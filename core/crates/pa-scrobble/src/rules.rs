// SPDX-License-Identifier: MPL-2.0
//! When a play counts (sans-IO), the rules of both services:
//!
//! - «now playing» as soon as a track plays;
//! - a scrobble once the track has been listened to for half its length or
//!   [`MAX_THRESHOLD_MS`] (4 min), whichever comes first;
//! - a track shorter than [`MIN_TRACK_MS`] (30 s) never counts; a track
//!   of unknown length counts after 4 minutes;
//! - only listening counts: the playhead moving while playing, no faster
//!   than the clock (a seek forward adds nothing, a pause stops the count);
//! - the same track again from the start (repeat one, previous) is a new
//!   play once the first one counted.
//!
//! ```
//! use pa_scrobble::rules::{Action, Tracker};
//! use pa_scrobble::Track;
//!
//! let track = Track { artist: "A".into(), title: "T".into(), duration_ms: Some(200_000), ..Track::default() };
//! let mut t = Tracker::default();
//! assert!(matches!(t.start("1", track, 0, 1_000_000)[..], [Action::NowPlaying(_)]));
//! assert!(t.progress(99_000, true, 1_099_000).is_empty());
//! assert!(matches!(t.progress(100_000, true, 1_100_000)[..], [Action::Scrobble(_)]));
//! ```

use crate::{Listen, Track};

/// Tracks shorter than this never count.
pub const MIN_TRACK_MS: u64 = 30_000;

/// A play counts after this much listening even if half the track is
/// longer.
pub const MAX_THRESHOLD_MS: u64 = 240_000;

/// Slack between the playhead and the clock (report jitter).
const SLACK_MS: u64 = 1_500;

/// Listening needed before a track of `duration_ms` counts; `None` for
/// tracks that never count.
pub fn threshold_ms(duration_ms: Option<u64>) -> Option<u64> {
    match duration_ms {
        Some(d) if d < MIN_TRACK_MS => None,
        Some(d) => Some((d / 2).min(MAX_THRESHOLD_MS)),
        None => Some(MAX_THRESHOLD_MS),
    }
}

/// What to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// The track started playing (not queued: a failed one is not retried).
    NowPlaying(Track),
    /// The play counts.
    Scrobble(Listen),
}

#[derive(Debug, Clone)]
struct Play {
    id: String,
    track: Track,
    /// When this play started, Unix ms.
    started_ms: u64,
    listened_ms: u64,
    /// Last report: playhead and clock.
    last: Option<(u64, u64)>,
    announced: bool,
    scrobbled: bool,
}

/// Follows the player and says when to send «now playing» and scrobbles.
#[derive(Debug, Default)]
pub struct Tracker {
    play: Option<Play>,
}

impl Tracker {
    /// The player moved to track `id` (a library id or anything stable) at
    /// `position_ms`; `now_ms` is Unix ms. Starting the track that already
    /// plays is a no-op.
    pub fn start(&mut self, id: &str, track: Track, position_ms: u64, now_ms: u64) -> Vec<Action> {
        if self.play.as_ref().is_some_and(|p| p.id == id) {
            return Vec::new();
        }
        if !track.is_complete() {
            self.play = None;
            return Vec::new();
        }
        self.play = Some(Play {
            id: id.to_owned(),
            track,
            started_ms: now_ms.saturating_sub(position_ms),
            listened_ms: 0,
            last: None,
            announced: false,
            scrobbled: false,
        });
        self.progress(position_ms, true, now_ms)
    }

    /// The playhead is at `position_ms` and the player is `playing` (or
    /// paused) at `now_ms`.
    pub fn progress(&mut self, position_ms: u64, playing: bool, now_ms: u64) -> Vec<Action> {
        let Some(play) = self.play.as_mut() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if let Some((pos, at)) = play.last {
            // The same track from the start again after it counted.
            if play.scrobbled && position_ms + 5_000 < pos && position_ms < 5_000 {
                play.started_ms = now_ms.saturating_sub(position_ms);
                play.listened_ms = 0;
                play.announced = false;
                play.scrobbled = false;
            } else if playing && position_ms > pos {
                let moved = position_ms - pos;
                let elapsed = now_ms.saturating_sub(at);
                if moved <= elapsed + SLACK_MS {
                    play.listened_ms += moved;
                }
            }
        }
        play.last = Some((position_ms, now_ms));
        if playing && !play.announced {
            play.announced = true;
            out.push(Action::NowPlaying(play.track.clone()));
        }
        if !play.scrobbled
            && let Some(need) = threshold_ms(play.track.duration_ms)
            && play.listened_ms >= need
        {
            play.scrobbled = true;
            out.push(Action::Scrobble(Listen {
                track: play.track.clone(),
                started_at: play.started_ms / 1000,
            }));
        }
        out
    }

    /// The player stopped or the queue is gone.
    pub fn stop(&mut self) {
        self.play = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_763_000_000_000;

    fn track(duration_ms: Option<u64>) -> Track {
        Track {
            artist: "Artist".into(),
            title: "Title".into(),
            duration_ms,
            ..Track::default()
        }
    }

    /// Plays from `from` to `to` in steps of 1 s.
    fn play(t: &mut Tracker, from: u64, to: u64, clock: &mut u64) -> Vec<Action> {
        let mut out = Vec::new();
        let mut pos = from;
        while pos < to {
            pos += 1_000;
            *clock += 1_000;
            out.extend(t.progress(pos, true, *clock));
        }
        out
    }

    fn scrobbles(a: &[Action]) -> usize {
        a.iter()
            .filter(|a| matches!(a, Action::Scrobble(_)))
            .count()
    }

    #[test]
    fn thresholds() {
        assert_eq!(threshold_ms(Some(29_999)), None);
        assert_eq!(threshold_ms(Some(30_000)), Some(15_000));
        assert_eq!(threshold_ms(Some(200_000)), Some(100_000));
        assert_eq!(threshold_ms(Some(600_000)), Some(240_000));
        assert_eq!(threshold_ms(None), Some(240_000));
    }

    #[test]
    fn half_of_a_short_track() {
        let mut t = Tracker::default();
        let mut clock = T0;
        let a = t.start("1", track(Some(200_000)), 0, clock);
        assert_eq!(a, vec![Action::NowPlaying(track(Some(200_000)))]);
        assert_eq!(scrobbles(&play(&mut t, 0, 99_000, &mut clock)), 0);
        let a = play(&mut t, 99_000, 100_000, &mut clock);
        assert_eq!(
            a,
            vec![Action::Scrobble(Listen {
                track: track(Some(200_000)),
                started_at: T0 / 1000,
            })]
        );
        assert_eq!(
            scrobbles(&play(&mut t, 100_000, 200_000, &mut clock)),
            0,
            "once"
        );
    }

    #[test]
    fn four_minutes_of_a_long_track_and_never_a_short_one() {
        let mut t = Tracker::default();
        let mut clock = T0;
        t.start("1", track(Some(20 * 60_000)), 0, clock);
        assert_eq!(scrobbles(&play(&mut t, 0, 239_000, &mut clock)), 0);
        assert_eq!(scrobbles(&play(&mut t, 239_000, 240_000, &mut clock)), 1);

        t.start("2", track(Some(25_000)), 0, clock);
        assert_eq!(scrobbles(&play(&mut t, 0, 25_000, &mut clock)), 0);
    }

    #[test]
    fn seeks_and_pauses_do_not_count() {
        let mut t = Tracker::default();
        let mut clock = T0;
        t.start("1", track(Some(200_000)), 0, clock);
        play(&mut t, 0, 10_000, &mut clock);
        // A seek to 150 s: one report later, the playhead jumped.
        clock += 1_000;
        assert!(t.progress(150_000, true, clock).is_empty());
        // Paused for a minute, the playhead still.
        clock += 60_000;
        assert!(t.progress(150_000, false, clock).is_empty());
        // Listened 10 + 40 s = 50 s: not yet.
        assert_eq!(scrobbles(&play(&mut t, 150_000, 190_000, &mut clock)), 0);
        // Back to 10 s and on to 60 s: 100 s listened in all.
        clock += 1_000;
        t.progress(10_000, true, clock);
        assert_eq!(scrobbles(&play(&mut t, 10_000, 60_000, &mut clock)), 1);
    }

    #[test]
    fn repeat_one_is_a_new_play_and_handoff_starts_midway() {
        let mut t = Tracker::default();
        let mut clock = T0;
        t.start("1", track(Some(60_000)), 0, clock);
        assert_eq!(scrobbles(&play(&mut t, 0, 60_000, &mut clock)), 1);
        clock += 500;
        let again = t.progress(500, true, clock);
        assert!(matches!(again[..], [Action::NowPlaying(_)]));
        assert_eq!(scrobbles(&play(&mut t, 500, 31_000, &mut clock)), 1);

        // Music handed over at 61 s: the play started 61 s ago.
        let mut t = Tracker::default();
        t.start("9", track(Some(200_000)), 61_000, T0);
        let mut clock = T0;
        let a = play(&mut t, 61_000, 161_000, &mut clock);
        assert_eq!(
            a,
            vec![Action::Scrobble(Listen {
                track: track(Some(200_000)),
                started_at: (T0 - 61_000) / 1000,
            })]
        );
    }

    #[test]
    fn incomplete_tracks_and_restarts_of_the_same_id() {
        let mut t = Tracker::default();
        let no_artist = Track {
            artist: " ".into(),
            ..track(Some(200_000))
        };
        assert!(t.start("1", no_artist, 0, T0).is_empty());
        assert!(t.progress(150_000, true, T0 + 150_000).is_empty());
        assert_eq!(t.start("2", track(Some(200_000)), 0, T0).len(), 1);
        assert!(t.start("2", track(Some(200_000)), 0, T0).is_empty());
        t.stop();
        assert!(t.progress(1_000, true, T0).is_empty());
    }
}
