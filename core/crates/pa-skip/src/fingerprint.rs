// SPDX-License-Identifier: MPL-2.0
//! Audio fingerprint: one 32-bit sub-fingerprint every 32 ms.
//!
//! The scheme is the band-energy-difference fingerprint of Haitsma and
//! Kalker ("A Highly Robust Audio Fingerprinting System", ISMIR 2002), our
//! own implementation: mono audio resampled to 8 kHz, 256 ms Hann frames
//! every 32 ms, energies of 33 logarithmic bands between 300 and 2000 Hz,
//! and bit `m` set when the energy difference of bands `m` and `m + 1`
//! grows from the previous frame. A frame is robust to re-encoding, level
//! and small time shifts (frames overlap by 7/8), so the same music in two
//! episodes gives sub-fingerprints a few bits apart.
//!
//! Silent frames are 0 and never match ([`SILENT`]).

/// Sample rate the fingerprint works at.
pub const RATE: u32 = 8000;
/// Frame length in samples (256 ms).
pub const FRAME: usize = 2048;
/// Step between frames in samples (32 ms).
pub const HOP: usize = 256;
/// Step between sub-fingerprints in milliseconds.
pub const HOP_MS: u64 = HOP as u64 * 1000 / RATE as u64;
/// Frame length in milliseconds.
pub const FRAME_MS: u64 = FRAME as u64 * 1000 / RATE as u64;
/// The sub-fingerprint of a silent (or the first) frame.
pub const SILENT: u32 = 0;

const BANDS: usize = 33;
const LOW_HZ: f64 = 300.0;
const HIGH_HZ: f64 = 2000.0;
/// Mean square of a frame below which it is silence (−60 dBFS).
const SILENCE: f32 = 1e-6;

/// Sub-fingerprints of a stretch of audio.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Fingerprint {
    /// Time of the first frame's start in the file.
    pub start_ms: u64,
    /// One per [`HOP_MS`]; [`SILENT`] for silence.
    pub hashes: Vec<u32>,
}

impl Fingerprint {
    /// Start time of frame `i` in the file.
    #[must_use]
    pub fn time_ms(&self, i: usize) -> u64 {
        self.start_ms + i as u64 * HOP_MS
    }

    /// Duration covered.
    #[must_use]
    pub fn duration_ms(&self) -> u64 {
        if self.hashes.is_empty() {
            0
        } else {
            (self.hashes.len() as u64 - 1) * HOP_MS + FRAME_MS
        }
    }

    /// Little-endian bytes for storage.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        self.hashes.iter().flat_map(|h| h.to_le_bytes()).collect()
    }

    /// From [`to_bytes`](Self::to_bytes); `None` if the length is not a
    /// multiple of 4.
    #[must_use]
    pub fn from_bytes(start_ms: u64, bytes: &[u8]) -> Option<Self> {
        if !bytes.len().is_multiple_of(4) {
            return None;
        }
        Some(Self {
            start_ms,
            hashes: bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| u32::from_le_bytes(*c))
                .collect(),
        })
    }

    /// Fingerprint of mono samples at `rate` starting at `start_ms`.
    #[must_use]
    pub fn of_samples(rate: u32, start_ms: u64, samples: &[f32]) -> Self {
        let mut b = Builder::new(start_ms);
        b.push(rate, samples);
        b.finish()
    }
}

/// Builds a [`Fingerprint`] from audio pushed in pieces.
#[derive(Debug)]
pub struct Builder {
    start_ms: u64,
    resampler: Option<Resampler>,
    frames: Framer,
    hashes: Vec<u32>,
}

impl Builder {
    /// A fingerprint whose first frame starts at `start_ms`.
    #[must_use]
    pub fn new(start_ms: u64) -> Self {
        Self {
            start_ms,
            resampler: None,
            frames: Framer::new(),
            hashes: Vec::new(),
        }
    }

    /// Adds mono samples at `rate` (any rate; a change of rate restarts
    /// the resampler without a gap in time).
    pub fn push(&mut self, rate: u32, samples: &[f32]) {
        if rate == 0 {
            return;
        }
        if self.resampler.as_ref().is_none_or(|r| r.in_rate != rate) {
            self.resampler = Some(Resampler::new(rate, RATE));
        }
        let Some(r) = self.resampler.as_mut() else {
            return;
        };
        let frames = &mut self.frames;
        let hashes = &mut self.hashes;
        r.push(samples, &mut |s| frames.push(s, hashes));
    }

    /// Number of sub-fingerprints so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.hashes.len()
    }

    /// Whether nothing was computed yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }

    #[must_use]
    pub fn finish(self) -> Fingerprint {
        Fingerprint {
            start_ms: self.start_ms,
            hashes: self.hashes,
        }
    }
}

/// Band-limited resampler (windowed sinc, table-driven).
#[derive(Debug)]
struct Resampler {
    in_rate: u32,
    /// Input samples per output sample.
    step: f64,
    /// Half-width of the kernel in input samples.
    half: f64,
    /// Kernel sampled every 1/[`PHASES`] input sample over `[-half, half]`.
    table: Vec<f32>,
    /// Buffered input; `buf[0]` is input sample `buf_start`.
    buf: Vec<f32>,
    buf_start: u64,
    /// Next output sample.
    next: u64,
}

const PHASES: f64 = 64.0;

impl Resampler {
    fn new(in_rate: u32, out_rate: u32) -> Self {
        let step = f64::from(in_rate) / f64::from(out_rate);
        // Cut off at 90 % of the lower Nyquist frequency, in cycles per
        // input sample.
        let cutoff = 0.45 * f64::from(in_rate.min(out_rate)) / f64::from(in_rate);
        let half = (4.0 / cutoff / 2.0).ceil().max(4.0);
        let n = (2.0 * half * PHASES) as usize + 1;
        let table = (0..n)
            .map(|i| {
                let x = i as f64 / PHASES - half;
                let sinc = if x.abs() < 1e-9 {
                    1.0
                } else {
                    let a = std::f64::consts::PI * 2.0 * cutoff * x;
                    a.sin() / a
                };
                let w = 0.5 + 0.5 * (std::f64::consts::PI * x / half).cos();
                (2.0 * cutoff * sinc * w) as f32
            })
            .collect();
        Self {
            in_rate,
            step,
            half,
            table,
            buf: Vec::new(),
            buf_start: 0,
            next: 0,
        }
    }

    fn push(&mut self, input: &[f32], out: &mut dyn FnMut(f32)) {
        self.buf.extend_from_slice(input);
        let end = self.buf_start + self.buf.len() as u64;
        loop {
            let t = self.next as f64 * self.step;
            let hi = (t + self.half).floor();
            if hi < 0.0 || hi as u64 >= end {
                break;
            }
            let lo = (t - self.half).ceil().max(0.0) as u64;
            let mut acc = 0.0f32;
            for k in lo.max(self.buf_start)..=hi as u64 {
                let x = k as f64 - t + self.half;
                let idx = (x * PHASES).round() as usize;
                if let Some(h) = self.table.get(idx) {
                    acc += self.buf[(k - self.buf_start) as usize] * h;
                }
            }
            out(acc);
            self.next += 1;
        }
        // Keep what the next output still needs.
        let keep_from = ((self.next as f64 * self.step - self.half).ceil().max(0.0) as u64)
            .max(self.buf_start)
            .min(end);
        let drop = (keep_from - self.buf_start) as usize;
        if drop > 0 {
            self.buf.drain(..drop);
            self.buf_start = keep_from;
        }
    }
}

/// Cuts 8 kHz audio into overlapping frames and fingerprints them.
#[derive(Debug)]
struct Framer {
    /// The last [`FRAME`] samples, circular; the oldest at `pos`.
    ring: Vec<f32>,
    pos: usize,
    /// Samples pushed so far.
    total: u64,
    window: Vec<f32>,
    /// FFT bins of each band edge.
    edges: [usize; BANDS + 1],
    prev: Option<[f64; BANDS]>,
    fft: Fft,
    re: Vec<f32>,
    im: Vec<f32>,
}

impl Framer {
    fn new() -> Self {
        let window = (0..FRAME)
            .map(|i| {
                let x = std::f32::consts::PI * 2.0 * i as f32 / FRAME as f32;
                0.5 - 0.5 * x.cos()
            })
            .collect();
        let mut edges = [0usize; BANDS + 1];
        for (i, e) in edges.iter_mut().enumerate() {
            let hz = LOW_HZ * (HIGH_HZ / LOW_HZ).powf(i as f64 / BANDS as f64);
            *e = (hz * FRAME as f64 / f64::from(RATE)).round() as usize;
        }
        Self {
            ring: vec![0.0; FRAME],
            pos: 0,
            total: 0,
            window,
            edges,
            prev: None,
            fft: Fft::new(FRAME),
            re: vec![0.0; FRAME],
            im: vec![0.0; FRAME],
        }
    }

    fn push(&mut self, sample: f32, hashes: &mut Vec<u32>) {
        self.ring[self.pos] = sample;
        self.pos = (self.pos + 1) % FRAME;
        self.total += 1;
        if self.total >= FRAME as u64 && (self.total - FRAME as u64).is_multiple_of(HOP as u64) {
            hashes.push(self.frame());
        }
    }

    fn frame(&mut self) -> u32 {
        let mut power = 0.0f32;
        for i in 0..FRAME {
            let s = self.ring[(self.pos + i) % FRAME];
            power += s * s;
            self.re[i] = s * self.window[i];
            self.im[i] = 0.0;
        }
        let silent = power / (FRAME as f32) < SILENCE;
        self.fft.run(&mut self.re, &mut self.im);
        let mut energy = [0.0f64; BANDS];
        for (b, e) in energy.iter_mut().enumerate() {
            for k in self.edges[b]..self.edges[b + 1].max(self.edges[b] + 1) {
                *e += f64::from(self.re[k] * self.re[k] + self.im[k] * self.im[k]);
            }
        }
        let prev = self.prev.replace(energy);
        let Some(prev) = prev else {
            return SILENT;
        };
        if silent {
            return SILENT;
        }
        let mut bits = 0u32;
        for m in 0..32 {
            let d = (energy[m] - energy[m + 1]) - (prev[m] - prev[m + 1]);
            if d > 0.0 {
                bits |= 1 << m;
            }
        }
        if bits == SILENT { 1 } else { bits }
    }
}

/// Radix-2 complex FFT of a fixed power-of-two size.
#[derive(Debug)]
struct Fft {
    n: usize,
    cos: Vec<f32>,
    sin: Vec<f32>,
    rev: Vec<usize>,
}

impl Fft {
    fn new(n: usize) -> Self {
        assert!(n.is_power_of_two());
        let bits = n.trailing_zeros();
        let rev = (0..n)
            .map(|i| i.reverse_bits() >> (usize::BITS - bits))
            .collect();
        let (cos, sin) = (0..n / 2)
            .map(|i| {
                let a = -2.0 * std::f64::consts::PI * i as f64 / n as f64;
                (a.cos() as f32, a.sin() as f32)
            })
            .unzip();
        Self { n, cos, sin, rev }
    }

    fn run(&self, re: &mut [f32], im: &mut [f32]) {
        let n = self.n;
        for i in 0..n {
            let j = self.rev[i];
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let half = len / 2;
            let stride = n / len;
            for start in (0..n).step_by(len) {
                for k in 0..half {
                    let (wr, wi) = (self.cos[k * stride], self.sin[k * stride]);
                    let (a, b) = (start + k, start + k + half);
                    let tr = re[b] * wr - im[b] * wi;
                    let ti = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len <<= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_finds_a_tone() {
        let fft = Fft::new(64);
        let mut re: Vec<f32> = (0..64)
            .map(|i| (2.0 * std::f32::consts::PI * 5.0 * i as f32 / 64.0).cos())
            .collect();
        let mut im = vec![0.0; 64];
        fft.run(&mut re, &mut im);
        let mags: Vec<f32> = re.iter().zip(&im).map(|(r, i)| r.hypot(*i)).collect();
        let peak = (0..32).max_by(|a, b| mags[*a].total_cmp(&mags[*b])).unwrap();
        assert_eq!(peak, 5);
        assert!((mags[5] - 32.0).abs() < 1e-3);
    }

    #[test]
    fn resampler_keeps_the_band_and_count() {
        let rate = 48_000;
        let tone: Vec<f32> = (0..rate)
            .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / rate as f32).sin())
            .collect();
        let mut r = Resampler::new(rate, RATE);
        let mut out = Vec::new();
        for chunk in tone.chunks(1000) {
            r.push(chunk, &mut |s| out.push(s));
        }
        // All but the kernel's half-width at the end.
        assert!(out.len() > 7_990 && out.len() <= 8_000, "{}", out.len());
        // 1 kHz passes at full level.
        let rms = (out[100..7900].iter().map(|s| s * s).sum::<f32>() / 7800.0).sqrt();
        assert!((rms - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.03, "{rms}");
        // 10 kHz (above 4 kHz Nyquist) is removed.
        let high: Vec<f32> = (0..rate)
            .map(|i| (2.0 * std::f32::consts::PI * 10_000.0 * i as f32 / rate as f32).sin())
            .collect();
        let mut r = Resampler::new(rate, RATE);
        let mut out = Vec::new();
        r.push(&high, &mut |s| out.push(s));
        let rms = (out[100..7900].iter().map(|s| s * s).sum::<f32>() / 7800.0).sqrt();
        assert!(rms < 0.02, "{rms}");
    }

    #[test]
    fn silence_is_silent_and_bytes_round_trip() {
        let fp = Fingerprint::of_samples(RATE, 1000, &vec![0.0; 8000]);
        assert!(!fp.hashes.is_empty());
        assert!(fp.hashes.iter().all(|h| *h == SILENT));
        let fp = Fingerprint {
            start_ms: 5,
            hashes: vec![1, 0xdead_beef, 0],
        };
        assert_eq!(Fingerprint::from_bytes(5, &fp.to_bytes()), Some(fp.clone()));
        assert_eq!(Fingerprint::from_bytes(5, &[1, 2, 3]), None);
        assert_eq!(fp.time_ms(2), 5 + 2 * HOP_MS);
        assert_eq!(HOP_MS, 32);
        assert_eq!(FRAME_MS, 256);
    }
}
