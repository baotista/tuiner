//! Level and Clarity gating into three states, plus the noise-floor tracker of ADR 0005.
//!
//! Neither gate alone is sufficient — measured, not assumed: silence below -60 dBFS reaches a
//! Clarity of 0.94, so Clarity alone cannot reject silence; chords sit inside the note Level
//! range, so Level alone cannot reject chords.
//!
//! ```text
//! Level below gate             -> Silent      hold last reading, dimmed
//! Level ok, Clarity < 0.90     -> Unpitched   no reading at all
//! both ok                      -> Pitched     live
//! ```

/// Below this Clarity a reading is not trusted, even with Level ok. Pinned by real-audio
/// measurement (ADR 0001): the originally assumed 0.8 sat at the median of real noise and let
/// a third of chord frames through; 0.90 keeps 91.3% of note frames while admitting only 4.1%
/// of chord frames.
pub const CLARITY_THRESHOLD: f32 = 0.90;

/// Level gate margin above the tracked noise floor, in dB. Adopted per ADR 0005.
pub const LEVEL_MARGIN_DB: f32 = 18.0;

/// Level gate ceiling, in dBFS — never gates below this even with no floor observed yet.
/// Co-equal to the tracking, not a safety net: if the app starts mid-performance there is no
/// quiet frame to learn the floor from, and the ceiling is the only thing keeping it from being
/// deaf until one arrives.
pub const LEVEL_CEILING_DB: f32 = -50.0;

/// How fast the tracked floor is allowed to drift upward, in dB/s — covers a genuinely noisier
/// environment without following a loud passage.
pub const LEVEL_LEAK_DB_PER_SEC: f32 = 0.1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Level below the gate. Caller should hold the last Pitched reading, dimmed.
    Silent,
    /// Level ok but Clarity too low to trust — no reading at all.
    Unpitched,
    /// Both gates passed — a Pitch is live.
    Pitched,
}

/// Classifies one frame from its Level and Clarity against the currently tracked gate.
pub fn classify(level_db: f32, gate_db: f32, clarity: f32) -> State {
    if level_db < gate_db {
        State::Silent
    } else if clarity < CLARITY_THRESHOLD {
        State::Unpitched
    } else {
        State::Pitched
    }
}

/// RMS Level of a window, in dBFS (0 dBFS = a full-scale sine at amplitude 1.0).
pub fn level_db(samples: &[f32]) -> f32 {
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    if rms > 0.0 {
        20.0 * rms.log10()
    } else {
        -120.0
    }
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
    leak_db_per_frame: f32,
}

impl NoiseFloor {
    /// `fps` is the only thing that varies by call site (it depends on hop timing); the
    /// margin, ceiling and leak rate are the ADR-0005-pinned constants above, not knobs —
    /// every call site would otherwise pass the same three values.
    pub fn new(fps: f32) -> Self {
        Self {
            floor_db: None,
            leak_db_per_frame: LEVEL_LEAK_DB_PER_SEC / fps.max(1.0),
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
        (self.floor_db() + LEVEL_MARGIN_DB).min(LEVEL_CEILING_DB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LOWPASS_MULT, MAX_HZ, MIN_HZ, detect, window_samples};

    fn floor() -> NoiseFloor {
        NoiseFloor::new(47.6)
    }

    #[test]
    fn ceiling_keeps_the_app_responsive_with_no_quiet_frame_observed() {
        let mut f = floor();
        // The player is already playing when the app starts — every observed frame is loud,
        // so there is no quiet frame to learn the true noise floor from. The very first
        // observation becomes the tracked "floor" (there is nothing lower yet to compare
        // against), which would push the gate well above -50dBFS without the ceiling.
        for _ in 0..100 {
            f.observe(-28.0);
        }
        assert_eq!(
            f.gate_db(),
            LEVEL_CEILING_DB,
            "with only loud frames ever observed, the ceiling must cap the gate at {LEVEL_CEILING_DB}, got {}",
            f.gate_db()
        );
        // A quieter moment within the same performance must still register as signal.
        assert_eq!(classify(-45.0, f.gate_db(), 0.95), State::Pitched);
    }

    #[test]
    fn sustained_loud_passage_does_not_drag_the_floor_upward() {
        let mut f = floor();
        f.observe(-75.0); // one quiet frame establishes the floor
        for _ in 0..1000 {
            f.observe(-20.0); // ~20s of continuous, loud playing
        }
        assert!(
            f.floor_db() < -60.0,
            "a sustained loud passage dragged the tracked floor up to {:.1} dBFS",
            f.floor_db()
        );
    }

    #[test]
    fn silent_when_level_is_below_the_gate() {
        assert_eq!(classify(-70.0, LEVEL_CEILING_DB, 0.99), State::Silent);
    }

    #[test]
    fn unpitched_when_level_ok_but_clarity_too_low() {
        assert_eq!(classify(-30.0, LEVEL_CEILING_DB, 0.5), State::Unpitched);
    }

    /// A crude deterministic PRNG standing in for a broadband click — no periodicity for the
    /// coarse or refined stage to lock onto, unlike a plucked string's harmonic stack.
    fn pseudo_noise(n: usize, mut state: u64) -> Vec<f32> {
        (0..n)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state as f64 / u64::MAX as f64) as f32 * 2.0 - 1.0
            })
            .collect()
    }

    #[test]
    fn broadband_transient_is_rejected_by_clarity_alone_with_no_attack_specific_code() {
        // Stands in for a pick attack: loud (so Level cannot be what rejects it) and
        // non-periodic (so only Clarity has any chance of catching it).
        let sample_rate = 48_000.0;
        let window = window_samples(sample_rate as u32);
        let mut det = detect::Detector::new(sample_rate, window, MIN_HZ, MAX_HZ)
            .with_lowpass(Some(LOWPASS_MULT));

        let transient = pseudo_noise(window, 0xDEAD_BEEF_1234_5678);
        let level = level_db(&transient);
        let clarity = det.analyse(&transient).map(|r| r.clarity).unwrap_or(0.0);

        let f = floor();
        assert!(
            level > f.gate_db(),
            "test signal must be loud enough that Level alone would pass it (got {level:.1} dBFS)"
        );
        assert_eq!(
            classify(level, f.gate_db(), clarity),
            State::Unpitched,
            "broadband transient at {level:.1} dBFS, clarity {clarity:.2} was not rejected by Clarity"
        );
    }
}
