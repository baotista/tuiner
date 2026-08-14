//! Median-3 then an EMA, so an isolated outlier frame never reaches the display.
//!
//! The median is what suppresses residual chord/noise frames that slip past the Clarity gate —
//! ADR 0001 and ADR 0005 both note they arrive isolated, which is exactly what a 3-tap median
//! eliminates outright rather than merely damping.

/// EMA weight given to each new (already median-filtered) value. Unmeasured, unlike the gate
/// and detector constants — no ADR pins a time constant for this. Chosen for a ~40ms settle
/// time at the pipeline's ~48fps hop rate, favouring responsiveness over extra steadiness; a
/// provisional default like the other constants here, not a physical one.
const EMA_ALPHA: f32 = 0.3;

/// A jump this large is treated as a new Pitch starting, not jitter to ease into — below one
/// semitone (100¢), so it never fires on ordinary detector noise (worst case ~1¢, ADR 0001),
/// but a legato transition between two different Strings, e.g. bass low E's E1↔E2, is 1200¢ and
/// always snaps. Without this, blending in Hz gives a smoothed value that glides through every
/// intermediate Note on the way — E1 to E2 by way of A1 — for as long as the EMA takes to settle.
const SNAP_THRESHOLD_CENTS: f32 = 50.0;

/// Smooths a stream of values from a single run of Pitched frames. Reset between runs — it is
/// not meant to blend across a gap where the Pitch could have changed to something unrelated.
pub struct Smoother {
    history: [f32; 2],
    filled: usize,
    ema: Option<f32>,
}

impl Default for Smoother {
    fn default() -> Self {
        Self::new()
    }
}

impl Smoother {
    pub fn new() -> Self {
        Self {
            history: [0.0; 2],
            filled: 0,
            ema: None,
        }
    }

    /// Drops all in-flight history. Call this whenever a Pitched run ends, so the next one
    /// starts clean instead of being median'd or eased against an unrelated previous Pitch.
    pub fn reset(&mut self) {
        self.filled = 0;
        self.ema = None;
    }

    /// Feeds one new value, returning the smoothed value for this frame.
    pub fn push(&mut self, value: f32) -> f32 {
        // The median runs first and unconditionally: it is what tells a single-frame outlier
        // apart from a real, sustained jump. A lone spurious value never moves the median (two
        // of the last three samples still agree on the old Pitch), so it never even reaches
        // the snap check below. A jump that persists for two frames does move the median, and
        // that is exactly when it should snap rather than wait out the EMA's slow decay.
        let median = if self.filled < 2 {
            self.history[self.filled] = value;
            self.filled += 1;
            value
        } else {
            let m = median3(self.history[0], self.history[1], value);
            self.history[0] = self.history[1];
            self.history[1] = value;
            m
        };

        let smoothed = match self.ema {
            None => median,
            Some(prev) => {
                let cents = 1200.0 * (median / prev).log2();
                if cents.abs() > SNAP_THRESHOLD_CENTS {
                    median
                } else {
                    EMA_ALPHA * median + (1.0 - EMA_ALPHA) * prev
                }
            }
        };
        self.ema = Some(smoothed);
        smoothed
    }
}

fn median3(a: f32, b: f32, c: f32) -> f32 {
    let mut v = [a, b, c];
    v.sort_by(|x, y| x.partial_cmp(y).unwrap());
    v[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_outlier_frame_never_reaches_the_output() {
        let mut s = Smoother::new();
        for _ in 0..5 {
            s.push(300.0);
        }
        let out = s.push(900.0); // a single-frame spike, e.g. a chord frame that slipped the gate
        assert!(
            (out - 300.0).abs() < 1.0,
            "an isolated outlier reached the output: {out}"
        );
        // And it stays gone on the following steady frame too.
        let out = s.push(300.0);
        assert!(
            (out - 300.0).abs() < 1.0,
            "outlier echoed into the next frame: {out}"
        );
    }

    #[test]
    fn a_large_sustained_jump_snaps_within_a_couple_of_frames_instead_of_gliding() {
        // Standing in for bass low E's E1 (41.2 Hz) to E2 (82.4 Hz) transition, without a gap
        // that would otherwise reset the smoother. The median needs two consecutive frames at
        // the new Pitch to outvote the old one (that's what keeps a single-frame outlier from
        // triggering this same path) — by the second frame it must snap outright rather than
        // still be gliding through the EMA's slow decay.
        let mut s = Smoother::new();
        for _ in 0..5 {
            s.push(41.2);
        }
        s.push(82.4);
        let out = s.push(82.4);
        assert!(
            (out - 82.4).abs() < 0.5,
            "a genuine, sustained Pitch jump glided instead of snapping: {out}"
        );
        for _ in 0..5 {
            let out = s.push(82.4);
            assert!(
                (out - 82.4).abs() < 0.5,
                "settled value drifted after the snap: {out}"
            );
        }
    }

    #[test]
    fn reset_drops_history_for_a_fresh_run() {
        let mut s = Smoother::new();
        for _ in 0..5 {
            s.push(100.0);
        }
        s.reset();
        // No blending with the pre-reset value — the first push of a new run passes through.
        let out = s.push(400.0);
        assert_eq!(out, 400.0);
    }
}
