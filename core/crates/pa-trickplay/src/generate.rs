// SPDX-License-Identifier: MPL-2.0
//! Thumbnail generation: keyframes at a fixed interval, several grabbers in
//! parallel, coarse to fine.
//!
//! The positions are visited coarse to fine (every 32nd, then the 16th in
//! between, …), so after the first few percent of the work the whole
//! timeline already has a preview near every point and [`LiveFrames::nearest`]
//! answers from the closest finished thumbnail. Each worker opens its own
//! [`FrameGrabber`] over the shared source: one seek and one intra frame per
//! thumbnail, which is what keeps a two-hour film on SMB within a minute.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use pa_probe::{Ffmpeg, Grab, GrabOptions, ProbeError, ProbeErrorKind};
use pa_vfs::ByteSource;

use crate::{Error, bif, jpeg};

/// How thumbnails are generated.
#[derive(Debug, Clone)]
pub struct GenerateOptions {
    /// Wanted time between thumbnails (Jellyfin's default: 10 s).
    pub interval_ms: u64,
    /// At most this many thumbnails; longer media get a longer interval.
    pub max_frames: usize,
    /// Thumbnail width in pixels (Jellyfin and Plex use 320).
    pub width: u32,
    /// JPEG quality, 1–100.
    pub quality: u8,
    /// Parallel grabbers (each with a one-thread decoder).
    pub workers: usize,
    /// Set to `true` from another thread to stop.
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for GenerateOptions {
    fn default() -> Self {
        Self {
            interval_ms: 10_000,
            max_frames: 1200,
            width: 320,
            quality: 70,
            workers: std::thread::available_parallelism()
                .map_or(2, std::num::NonZero::get)
                .clamp(1, 4),
            cancel: None,
        }
    }
}

impl GenerateOptions {
    fn is_cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
    }
}

/// Interval and number of thumbnails for `duration_ms`: the wanted interval,
/// or a longer one (whole seconds) if that would exceed `max_frames`.
#[must_use]
pub fn plan(duration_ms: u64, opts: &GenerateOptions) -> (u64, usize) {
    let mut interval = opts.interval_ms.max(100);
    let max = opts.max_frames.max(1) as u64;
    if duration_ms.div_ceil(interval) > max {
        interval = duration_ms.div_ceil(max).div_ceil(1000) * 1000;
    }
    let count = duration_ms.div_ceil(interval).max(1);
    (interval, usize::try_from(count).unwrap_or(usize::MAX))
}

/// Indices `0..count` coarse to fine: multiples of 32 first, then of 16,
/// down to every index.
#[must_use]
pub fn coarse_to_fine(count: usize) -> Vec<usize> {
    let mut order = Vec::with_capacity(count);
    let mut stride = 32usize;
    loop {
        order.extend((0..count).step_by(stride).filter(|i| {
            // Not already taken by a coarser stride.
            stride == 32 || i % (stride * 2) != 0
        }));
        if stride == 1 {
            break;
        }
        stride /= 2;
    }
    order
}

#[derive(Clone)]
struct Slot {
    jpeg: Arc<[u8]>,
    position_ms: u64,
}

/// Thumbnails as they are being generated; readable at any time.
pub struct LiveFrames {
    interval_ms: u64,
    slots: Mutex<Vec<Option<Slot>>>,
    filled: AtomicUsize,
}

impl std::fmt::Debug for LiveFrames {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveFrames")
            .field("interval_ms", &self.interval_ms)
            .field("filled", &self.filled())
            .field("total", &self.total())
            .finish()
    }
}

impl LiveFrames {
    /// Empty slots for `count` thumbnails `interval_ms` apart.
    #[must_use]
    pub fn new(interval_ms: u64, count: usize) -> Self {
        Self {
            interval_ms: interval_ms.max(1),
            slots: Mutex::new(vec![None; count]),
            filled: AtomicUsize::new(0),
        }
    }

    fn slots(&self) -> std::sync::MutexGuard<'_, Vec<Option<Slot>>> {
        self.slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Time between thumbnails.
    #[must_use]
    pub fn interval_ms(&self) -> u64 {
        self.interval_ms
    }

    /// Number of slots.
    #[must_use]
    pub fn total(&self) -> usize {
        self.slots().len()
    }

    /// Slots done (with a thumbnail or known to have none).
    #[must_use]
    pub fn filled(&self) -> usize {
        self.filled.load(Ordering::Relaxed)
    }

    fn put(&self, index: usize, slot: Option<Slot>) {
        let mut slots = self.slots();
        if let Some(s) = slots.get_mut(index) {
            if s.is_none() {
                self.filled.fetch_add(1, Ordering::Relaxed);
            }
            if slot.is_some() {
                *s = slot;
            }
        }
    }

    /// The finished thumbnail closest to `position_ms` (its slot first):
    /// `(slot index, JPEG, keyframe time)`.
    #[must_use]
    pub fn nearest(&self, position_ms: u64) -> Option<(usize, Arc<[u8]>, u64)> {
        let slots = self.slots();
        if slots.is_empty() {
            return None;
        }
        let want = usize::try_from(position_ms / self.interval_ms)
            .unwrap_or(usize::MAX)
            .min(slots.len() - 1);
        let hit = |i: usize| {
            slots[i]
                .as_ref()
                .map(|s| (i, Arc::clone(&s.jpeg), s.position_ms))
        };
        for d in 0..slots.len() {
            if let Some(found) = want.checked_sub(d).and_then(hit) {
                return Some(found);
            }
            if want + d < slots.len()
                && let Some(found) = hit(want + d)
            {
                return Some(found);
            }
        }
        None
    }

    /// The thumbnails as a BIF file; empty slots repeat the previous
    /// thumbnail's index entry by being left out.
    #[must_use]
    pub fn to_bif(&self) -> Vec<u8> {
        let slots = self.slots();
        let frames: Vec<(u32, &[u8])> = slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| Some((u32::try_from(i).ok()?, &*s.as_ref()?.jpeg)))
            .collect();
        bif::write(u32::try_from(self.interval_ms).unwrap_or(u32::MAX), &frames)
    }
}

/// What a finished generation did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Generated {
    /// Thumbnails decoded and encoded.
    pub decoded: usize,
    /// Slots that reuse the keyframe of another slot (long GOPs).
    pub reused: usize,
    /// Slots without a keyframe (end of a file with a too long duration).
    pub missing: usize,
}

/// Opens the first grabber and plans the work: the live table to fill.
/// Marks `source` for random access ([`ByteSource::hint_random_access`]),
/// so it should be a handle of its own, not one shared with playback.
///
/// # Errors
///
/// [`Error::Probe`] if the file cannot be opened for decoding,
/// [`Error::Invalid`] if it has no duration.
pub fn prepare(
    ffmpeg: &Ffmpeg,
    source: &Arc<dyn ByteSource>,
    opts: &GenerateOptions,
) -> Result<Arc<LiveFrames>, Error> {
    // Keyframes are short reads all over the file: no playback read-ahead.
    source.hint_random_access();
    let grabber = ffmpeg
        .frame_grabber(Arc::clone(source), grab_options(opts))
        .map_err(probe_error)?;
    let duration = grabber
        .duration_ms()
        .ok_or_else(|| Error::Invalid("unknown duration".to_owned()))?;
    let (interval, count) = plan(duration, opts);
    Ok(Arc::new(LiveFrames::new(interval, count)))
}

/// A probe error, with cancellation as [`Error::Cancelled`].
fn probe_error(e: ProbeError) -> Error {
    if e.kind() == ProbeErrorKind::Cancelled {
        Error::Cancelled
    } else {
        Error::Probe(e)
    }
}

fn grab_options(opts: &GenerateOptions) -> GrabOptions {
    GrabOptions {
        width: opts.width,
        threads: 1,
        cancel: opts.cancel.clone(),
        ..GrabOptions::default()
    }
}

/// Fills `live` (from [`prepare`]) with thumbnails of `source`. Blocking;
/// returns when every slot is done, on cancellation or after repeated
/// failures.
///
/// # Errors
///
/// [`Error::Probe`] with the first error if no grabber opens or too many
/// grabs fail; [`Error::Cancelled`].
pub fn run(
    ffmpeg: &Ffmpeg,
    source: &Arc<dyn ByteSource>,
    opts: &GenerateOptions,
    live: &LiveFrames,
) -> Result<Generated, Error> {
    let total = live.total();
    let order = coarse_to_fine(total);
    let next = AtomicUsize::new(0);
    let keyframes: Mutex<HashMap<u64, Arc<[u8]>>> = Mutex::new(HashMap::new());
    let failure: Mutex<Option<Error>> = Mutex::new(None);
    let stop = AtomicBool::new(false);
    let decoded = AtomicUsize::new(0);
    let reused = AtomicUsize::new(0);
    let missing = AtomicUsize::new(0);
    let failures = AtomicUsize::new(0);
    let workers = opts.workers.clamp(1, 16).min(total.max(1));
    let interval = live.interval_ms();

    let fail = |e: Error| {
        stop.store(true, Ordering::Relaxed);
        failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_or_insert(e);
    };
    let known = |pos: u64| {
        keyframes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&pos)
            .cloned()
    };

    let work = || {
        let mut grabber = match ffmpeg.frame_grabber(Arc::clone(source), grab_options(opts)) {
            Ok(g) => g,
            Err(e) => return fail(probe_error(e)),
        };
        loop {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            if opts.is_cancelled() {
                return fail(Error::Cancelled);
            }
            let k = next.fetch_add(1, Ordering::Relaxed);
            let Some(&index) = order.get(k) else { return };
            let at = index as u64 * interval;
            match grabber.grab(at, |pos| known(pos).is_some()) {
                Ok(Grab::Frame(frame)) => match jpeg::encode(&frame, opts.quality) {
                    Ok(bytes) => {
                        let bytes: Arc<[u8]> = bytes.into();
                        keyframes
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .insert(frame.position_ms, Arc::clone(&bytes));
                        live.put(
                            index,
                            Some(Slot {
                                jpeg: bytes,
                                position_ms: frame.position_ms,
                            }),
                        );
                        decoded.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => return fail(Error::Encode(e)),
                },
                Ok(Grab::Same(pos)) => {
                    let slot = known(pos).map(|jpeg| Slot {
                        jpeg,
                        position_ms: pos,
                    });
                    live.put(index, slot);
                    reused.fetch_add(1, Ordering::Relaxed);
                }
                Ok(Grab::End) => {
                    live.put(index, None);
                    missing.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) if e.kind() == ProbeErrorKind::Cancelled => return fail(Error::Cancelled),
                Err(e) => {
                    live.put(index, None);
                    // A damaged spot is skipped; a broken source stops us.
                    if failures.fetch_add(1, Ordering::Relaxed) >= 8 {
                        return fail(probe_error(e));
                    }
                }
            }
        }
    };

    std::thread::scope(|s| {
        for _ in 1..workers {
            s.spawn(work);
        }
        work();
    });

    if let Some(e) = failure
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
    {
        return Err(e);
    }
    Ok(Generated {
        decoded: decoded.into_inner(),
        reused: reused.into_inner(),
        missing: missing.into_inner(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_interval_and_count() {
        let opts = GenerateOptions::default();
        // Two hours at 10 s.
        assert_eq!(plan(7_200_000, &opts), (10_000, 720));
        // Very long media: the interval grows to whole seconds.
        let (interval, count) = plan(10 * 3_600_000, &opts);
        assert_eq!(interval, 30_000);
        assert_eq!(count, 1200);
        // Shorter than one interval: one thumbnail.
        assert_eq!(plan(3_000, &opts), (10_000, 1));
        assert_eq!(plan(0, &opts), (10_000, 1));
    }

    #[test]
    fn coarse_to_fine_visits_everything_once() {
        for count in [0, 1, 2, 31, 33, 100, 720] {
            let order = coarse_to_fine(count);
            let mut sorted = order.clone();
            sorted.sort_unstable();
            assert_eq!(sorted, (0..count).collect::<Vec<_>>(), "count {count}");
        }
        let order = coarse_to_fine(100);
        assert_eq!(&order[..4], &[0, 32, 64, 96]);
        assert_eq!(&order[4..7], &[16, 48, 80]);
    }

    #[test]
    fn live_frames_answer_from_the_nearest() {
        let live = LiveFrames::new(10_000, 10);
        assert_eq!(live.nearest(50_000), None);
        let jpeg: Arc<[u8]> = Arc::from(&b"j0"[..]);
        live.put(
            0,
            Some(Slot {
                jpeg: Arc::clone(&jpeg),
                position_ms: 0,
            }),
        );
        live.put(
            8,
            Some(Slot {
                jpeg: Arc::from(&b"j8"[..]),
                position_ms: 79_000,
            }),
        );
        live.put(5, None);
        assert_eq!(live.filled(), 3);
        assert_eq!(live.nearest(15_000).map(|n| n.0), Some(0));
        assert_eq!(live.nearest(65_000).map(|n| n.0), Some(8));
        assert_eq!(
            live.nearest(1_000_000).map(|n| (n.0, n.2)),
            Some((8, 79_000))
        );
        let bif = bif::Bif::parse(live.to_bif()).unwrap();
        assert_eq!(bif.len(), 2);
        assert_eq!(bif.frame(1), Some((&b"j8"[..], 80_000)));
    }
}
