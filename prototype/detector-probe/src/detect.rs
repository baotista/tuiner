//! MPM coarse estimate + period-multiple refinement over an FFT autocorrelation.
//!
//! THIS MODULE IS THE PART WORTH KEEPING. `main.rs` around it is throwaway scaffolding.
//! It is deliberately pure: no audio I/O, no printing, no global state. Feed it a window of
//! mono samples, get a `Reading` back. That makes it liftable into the real crate once the
//! constants it exists to measure have been pinned.
//!
//! See ../../../docs/adr/0001-two-stage-pitch-detection.md for why it is shaped this way.

use realfft::RealFftPlanner;
use realfft::num_complex::Complex;

/// How far past the provisional cap of 30 we probe, so we can see *where* k actually fails
/// rather than merely confirming that 30 is allowed.
pub const K_PROBE_LIMIT: usize = 64;

/// Fraction of the window we let a lag reach. Beyond this, too few samples overlap for the
/// NSDF to mean anything, however well normalised it is.
const MAX_LAG_FRACTION: f32 = 0.75;

/// A k-multiple is considered "found" only if its peak is this periodic AND its implied
/// frequency agrees with the coarse estimate. Both matter: a tall peak at the wrong lag is
/// worse than no peak, because it silently reports a confident wrong pitch.
const K_FOUND_NSDF: f32 = 0.5;
const K_FOUND_CENTS: f32 = 50.0;

#[derive(Debug, Clone, Copy)]
pub struct KProbe {
    pub k: usize,
    /// NSDF height at the peak near lag ≈ k·T. This is the "findability" measure.
    pub nsdf: f32,
    pub implied_hz: f32,
    /// Disagreement with the coarse estimate. Large means we locked a neighbouring peak.
    pub cents_vs_coarse: f32,
    pub found: bool,
}

#[derive(Debug, Clone)]
pub struct Reading {
    pub coarse_hz: f32,
    pub coarse_lag: f32,
    /// Best refined estimate — the highest usable k.
    pub refined_hz: f32,
    pub k_used: usize,
    pub clarity: f32,
    /// Largest k whose peak was found. Limited by the signal.
    pub k_max_signal: usize,
    /// Largest k the window could physically hold. Limited by our window length, not physics.
    pub k_max_window: usize,
    pub probes: Vec<KProbe>,
}

pub struct Detector {
    sample_rate: f32,
    window: usize,
    fft_len: usize,
    min_lag: usize,
    /// Ceiling for the COARSE search: long enough to hold one period of the lowest note.
    max_lag: usize,
    /// Ceiling for REFINEMENT, which deliberately looks far past one period. Bounded only by
    /// how much of the window must still overlap for the NSDF to mean anything. Conflating
    /// this with `max_lag` clamps every probe to k=1 and makes refinement silently inert.
    refine_max_lag: usize,
    planner: RealFftPlanner<f32>,
    // Scratch, reused every frame so the hot path allocates nothing.
    padded: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    acf: Vec<f32>,
    prefix_sq: Vec<f32>,
    nsdf: Vec<f32>,
    /// |X|² kept aside, so the low-pass pass can re-use it without a second forward FFT.
    power: Vec<f32>,
    nsdf_lp: Vec<f32>,
    lowpass_mult: Option<f32>,
}

impl Detector {
    pub fn new(sample_rate: f32, window: usize, min_hz: f32, max_hz: f32) -> Self {
        let fft_len = (2 * window).next_power_of_two();
        // Both bounds carry margin. Without it the extremes of the range fail hard: a peak at
        // exactly `min_lag` is stepped over by the local-maximum scan (which starts at
        // min_lag+1) and the octave below is locked instead, and a peak at exactly `max_lag`
        // falls outside the scan's `t+1 < max_lag` guard. Measured as -1200¢ at E6 and no
        // detection at all at B0.
        const EDGE_MARGIN: usize = 3;
        let min_lag = ((sample_rate / max_hz).floor() as usize)
            .saturating_sub(EDGE_MARGIN)
            .max(2);
        let refine_max_lag = (window as f32 * MAX_LAG_FRACTION) as usize;
        let max_lag = (((sample_rate / min_hz).ceil() as usize) + EDGE_MARGIN).min(refine_max_lag);
        let mut planner = RealFftPlanner::<f32>::new();
        let fwd = planner.plan_fft_forward(fft_len);
        let padded = fwd.make_input_vec();
        let spectrum = fwd.make_output_vec();
        Self {
            sample_rate,
            window,
            fft_len,
            min_lag,
            max_lag,
            refine_max_lag,
            planner,
            padded,
            spectrum,
            acf: vec![0.0; fft_len],
            prefix_sq: vec![0.0; window + 1],
            nsdf: vec![0.0; window],
            power: vec![0.0; fft_len / 2 + 1],
            nsdf_lp: vec![0.0; window],
            lowpass_mult: None,
        }
    }

    /// Low-pass ahead of refinement, at `mult` × the coarse f0. The coarse pass only needs the
    /// right octave, so it stays full-band; refinement runs on a band-limited autocorrelation.
    /// Motivation: inharmonicity error is carried almost entirely by the upper partials, which
    /// pull the best-fit period sharp. Removing them removes most of the bias.
    pub fn with_lowpass(mut self, mult: Option<f32>) -> Self {
        self.lowpass_mult = mult;
        self
    }

    /// The longest lag refinement will look at, in samples — the ceiling on `k · T`.
    pub fn refine_max_lag(&self) -> usize {
        self.refine_max_lag
    }

    pub fn analyse(&mut self, samples: &[f32]) -> Option<Reading> {
        assert_eq!(samples.len(), self.window, "window size mismatch");

        // DC removal. A pickup or interface with any DC offset otherwise puts a huge
        // spurious component at lag 0 and skews every NSDF value.
        let mean = samples.iter().sum::<f32>() / self.window as f32;

        self.padded.fill(0.0);
        for (dst, &s) in self.padded.iter_mut().zip(samples) {
            *dst = s - mean;
        }

        // Autocorrelation via FFT: IFFT(|FFT(x)|²). Yields every lag in one pass, which is
        // exactly what the k-multiple stage needs.
        let fwd = self.planner.plan_fft_forward(self.fft_len);
        fwd.process(&mut self.padded, &mut self.spectrum).ok()?;
        for (p, c) in self.power.iter_mut().zip(self.spectrum.iter_mut()) {
            *p = c.norm_sqr();
            *c = Complex::new(*p, 0.0);
        }
        let inv = self.planner.plan_fft_inverse(self.fft_len);
        inv.process(&mut self.spectrum, &mut self.acf).ok()?;
        let scale = 1.0 / self.fft_len as f32;
        for v in self.acf.iter_mut() {
            *v *= scale;
        }

        // NSDF (McLeod): nsdf[t] = 2·acf[t] / m[t], where
        //   m[t] = Σ x[n]² + Σ x[n+t]²  over the overlapping region.
        // Obtained from prefix sums of squares rather than an O(N) pass per lag.
        self.prefix_sq[0] = 0.0;
        for i in 0..self.window {
            let x = samples[i] - mean;
            self.prefix_sq[i + 1] = self.prefix_sq[i] + x * x;
        }
        let total_sq = self.prefix_sq[self.window];
        if total_sq <= f32::EPSILON {
            return None;
        }
        for t in 0..self.window {
            let m = self.prefix_sq[self.window - t] + total_sq - self.prefix_sq[t];
            self.nsdf[t] = if m > f32::EPSILON {
                2.0 * self.acf[t] / m
            } else {
                0.0
            };
        }

        let coarse_lag = self.coarse_lag()?;
        let coarse_hz = self.sample_rate / coarse_lag;
        let clarity = parabolic(&self.nsdf, coarse_lag.round() as usize).1.clamp(0.0, 1.0);

        // Optional band-limited autocorrelation for stage 2 only. ACF of a filtered signal is
        // IFFT(|H|²·|X|²), so tapering the stored power spectrum gives it without a second
        // forward transform. The taper is a raised cosine rather than a brick wall, because an
        // abrupt cut rings and plants sidelobes right where we are about to look for peaks.
        let refine_on_lp = match self.lowpass_mult {
            Some(mult) => {
                let cutoff_hz = coarse_hz * mult;
                let bin_hz = self.sample_rate / self.fft_len as f32;
                let cut = cutoff_hz / bin_hz;
                let start = cut * 0.85;
                for (i, c) in self.spectrum.iter_mut().enumerate() {
                    let f = i as f32;
                    let h = if f <= start {
                        1.0
                    } else if f >= cut {
                        0.0
                    } else {
                        0.5 * (1.0 + (std::f32::consts::PI * (f - start) / (cut - start)).cos())
                    };
                    *c = Complex::new(self.power[i] * h * h, 0.0);
                }
                let inv = self.planner.plan_fft_inverse(self.fft_len);
                inv.process(&mut self.spectrum, &mut self.acf).ok()?;
                let z = self.acf[0];
                if z <= f32::EPSILON {
                    return None;
                }
                // Normalised without prefix sums: for a stationary signal m(τ) is proportional
                // to the overlap (1 − τ/N), so dividing that out approximates the NSDF closely
                // enough to locate peaks, which is all stage 2 needs from it.
                for t in 0..self.window {
                    let overlap = 1.0 - t as f32 / self.window as f32;
                    self.nsdf_lp[t] = if overlap > 1e-3 { self.acf[t] / (z * overlap) } else { 0.0 };
                }
                true
            }
            None => false,
        };
        let probe_curve: &[f32] = if refine_on_lp { &self.nsdf_lp } else { &self.nsdf };

        // Stage 2. Probe every k, including well past the provisional cap, and record what
        // happens rather than stopping at the first success.
        let k_max_window = ((self.refine_max_lag as f32 / coarse_lag).floor() as usize)
            .min(K_PROBE_LIMIT)
            .max(1);
        let mut probes = Vec::with_capacity(k_max_window);
        for k in 2..=k_max_window {
            let center = coarse_lag * k as f32;
            let half = (coarse_lag * 0.4).max(2.0);
            let lo = (center - half).floor() as usize;
            let hi = ((center + half).ceil() as usize).min(self.refine_max_lag.saturating_sub(1));
            if lo < 1 || hi <= lo + 1 {
                break;
            }
            let mut best = lo;
            for t in lo..=hi {
                if probe_curve[t] > probe_curve[best] {
                    best = t;
                }
            }
            let (peak_lag, peak_val) = parabolic(probe_curve, best);
            let implied_hz = self.sample_rate / (peak_lag / k as f32);
            let cents = 1200.0 * (implied_hz / coarse_hz).log2();
            probes.push(KProbe {
                k,
                nsdf: peak_val,
                implied_hz,
                cents_vs_coarse: cents,
                found: peak_val >= K_FOUND_NSDF && cents.abs() <= K_FOUND_CENTS,
            });
        }

        let best = probes.iter().rev().find(|p| p.found);
        let (refined_hz, k_used) = match best {
            Some(p) => (p.implied_hz, p.k),
            None => (coarse_hz, 1),
        };
        let k_max_signal = probes.iter().filter(|p| p.found).map(|p| p.k).max().unwrap_or(1);

        Some(Reading {
            coarse_hz,
            coarse_lag,
            refined_hz,
            k_used,
            clarity,
            k_max_signal,
            k_max_window,
            probes,
        })
    }

    /// McLeod pitch method: after the first positive-going zero crossing, take all local
    /// maxima, then pick the FIRST one within 90% of the tallest. Picking the tallest
    /// outright is what produces octave errors; this is the whole trick.
    fn coarse_lag(&self) -> Option<f32> {
        let mut start = self.min_lag;
        // Skip the descent from nsdf[0] = 1.
        let mut t = 1;
        while t < self.max_lag && self.nsdf[t] > 0.0 {
            t += 1;
        }
        while t < self.max_lag && self.nsdf[t] <= 0.0 {
            t += 1;
        }
        start = start.max(t);

        let mut maxima: Vec<usize> = Vec::new();
        let mut t = start + 1;
        while t + 1 < self.max_lag {
            if self.nsdf[t] > self.nsdf[t - 1] && self.nsdf[t] >= self.nsdf[t + 1] {
                maxima.push(t);
                // Skip to the end of this hump so a noisy plateau isn't 20 maxima.
                while t + 1 < self.max_lag && self.nsdf[t + 1] <= self.nsdf[t] {
                    t += 1;
                }
            }
            t += 1;
        }
        if maxima.is_empty() {
            return None;
        }
        let tallest = maxima
            .iter()
            .map(|&t| self.nsdf[t])
            .fold(f32::MIN, f32::max);
        if tallest <= 0.0 {
            return None;
        }
        let threshold = 0.9 * tallest;
        let chosen = maxima.iter().find(|&&t| self.nsdf[t] >= threshold)?;
        Some(parabolic(&self.nsdf, *chosen).0)
    }
}

/// Parabolic interpolation around an integer peak. Returns (sub-sample lag, peak height).
/// Without this, cent error at the top of our range is ±24 cents — see ADR 0001.
fn parabolic(y: &[f32], t: usize) -> (f32, f32) {
    if t == 0 || t + 1 >= y.len() {
        return (t as f32, y.get(t).copied().unwrap_or(0.0));
    }
    let (y0, y1, y2) = (y[t - 1], y[t], y[t + 1]);
    let denom = y0 - 2.0 * y1 + y2;
    if denom.abs() < f32::EPSILON {
        return (t as f32, y1);
    }
    let delta = 0.5 * (y0 - y2) / denom;
    let delta = delta.clamp(-1.0, 1.0);
    (t as f32 + delta, y1 - 0.25 * (y0 - y2) * delta)
}

const NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Nearest note plus deviation from it, at A4 = 440.
pub fn nearest_note(hz: f32) -> (String, f32) {
    let midi = 69.0 + 12.0 * (hz / 440.0).log2();
    let nearest = midi.round();
    let cents = (midi - nearest) * 100.0;
    let n = nearest as i32;
    let name = format!("{}{}", NAMES[(n.rem_euclid(12)) as usize], n / 12 - 1);
    (name, cents)
}

/// Tracks the noise floor so the Level gate adapts to the rig instead of hard-coding a dBFS
/// number. A fixed floor cannot work: the measured floor was -78 dBFS on one interface at one
/// gain setting, and turning the gain knob slides the whole distribution.
///
/// A running minimum over the whole session, not a sliding window and not an average:
///
/// - An **average** is dragged upward by loud passages. Play continuously and an EMA-based floor
///   climbs until it gates off the notes it exists to pass.
/// - A **sliding window** has the same flaw over its own length. Measured: a 10 s window during
///   continuous playing tracked the playing level (-27.9 dBFS), not the noise floor.
///
/// A noise floor is a property of the rig and gain setting, so it barely changes within a session.
/// The minimum captures it from the first quiet moment and is immune to loud passages by
/// construction, with a slow upward leak in case the environment genuinely gets noisier.
///
/// `ceiling_db` is not merely a safety net — it is co-equal to the tracking. If the app starts
/// while the player is already playing there is no quiet frame to learn from, and the ceiling is
/// the only thing keeping the app from being deaf until one arrives.
pub struct NoiseFloor {
    floor_db: Option<f32>,
    margin_db: f32,
    ceiling_db: f32,
    leak_db_per_frame: f32,
}

impl NoiseFloor {
    pub fn new(margin_db: f32, ceiling_db: f32, leak_db_per_sec: f32, fps: f32) -> Self {
        Self {
            floor_db: None,
            margin_db,
            ceiling_db,
            leak_db_per_frame: leak_db_per_sec / fps.max(1.0),
        }
    }

    pub fn observe(&mut self, db: f32) {
        self.floor_db = Some(match self.floor_db {
            None => db,
            Some(f) if db < f => db,
            Some(f) => f + self.leak_db_per_frame,
        });
    }

    pub fn floor_db(&self) -> f32 {
        self.floor_db.unwrap_or(-120.0)
    }

    /// The level a frame must exceed to be considered signal.
    pub fn gate_db(&self) -> f32 {
        (self.floor_db() + self.margin_db).min(self.ceiling_db)
    }
}
