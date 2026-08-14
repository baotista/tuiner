//! The Strobe's phase accumulator, per ADR 0002: the primary fine-precision indicator, because
//! encoding Deviation as rate of apparent motion rather than position gives it no floor — half a
//! cent flat is still visible as motion too slow to finish, unlike a bar too coarse to plot it.
//!
//! Small, but flagged by the ADR as stateful and dependent on frame timing rather than a pure
//! function of the latest reading — exactly what should be isolated and tested rather than
//! eyeballed. Rendering the phase into an actual banded pattern is `ui`'s job, not this module's;
//! this owns only the accumulator.

use std::f32::consts::TAU;

/// Advanced once per frame by `2π · (f_detected − f_target) · dt`, wrapped into `[0, TAU)`.
#[derive(Debug, Clone, Copy)]
pub struct Strobe {
    phase: f32,
}

impl Strobe {
    pub fn new() -> Self {
        Self { phase: 0.0 }
    }

    /// `dt_secs` is real elapsed time since the last call, not a fixed nominal frame period — a
    /// caller that stalls and then catches up passes a large `dt_secs`, which is exactly what
    /// turns into the documented phase jump rather than a panic.
    pub fn advance(&mut self, f_detected: f32, f_target: f32, dt_secs: f32) {
        self.phase = (self.phase + TAU * (f_detected - f_target) * dt_secs).rem_euclid(TAU);
    }

    pub fn phase(&self) -> f32 {
        self.phase
    }
}

impl Default for Strobe {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_stationary_at_zero_phase() {
        assert_eq!(Strobe::new().phase(), 0.0);
    }

    #[test]
    fn zero_deviation_never_moves_the_phase() {
        let mut s = Strobe::new();
        for _ in 0..100 {
            s.advance(196.0, 196.0, 0.021);
        }
        assert_eq!(s.phase(), 0.0);
    }

    #[test]
    fn sharp_deviation_advances_phase_positively() {
        let mut s = Strobe::new();
        s.advance(196.5, 196.0, 0.021); // detected above target: sharp
        assert!(
            s.phase() > 0.0 && s.phase() < std::f32::consts::PI,
            "expected a small positive phase, got {}",
            s.phase()
        );
    }

    #[test]
    fn flat_deviation_advances_phase_in_the_opposite_direction_from_sharp() {
        let mut s = Strobe::new();
        s.advance(195.5, 196.0, 0.021); // detected below target: flat
        // A negative phase increment wraps into the top half of [0, TAU) — the opposite side
        // from the small positive phase a sharp Deviation produces.
        assert!(
            s.phase() > std::f32::consts::PI,
            "expected a wrapped, opposite-direction phase, got {}",
            s.phase()
        );
    }

    #[test]
    fn drift_rate_is_proportional_to_the_deviation_in_hertz() {
        let mut single = Strobe::new();
        single.advance(196.05, 196.0, 1.0);
        let mut double = Strobe::new();
        double.advance(196.10, 196.0, 1.0);
        assert!(
            (double.phase() - 2.0 * single.phase()).abs() < 1e-4,
            "expected double the Deviation to advance twice as far: {} vs {}",
            single.phase(),
            double.phase()
        );
    }

    /// The number ADR 0002 and issue #6 both cite: half a cent flat on G3 (≈196.00 Hz) should
    /// take roughly 18 seconds to complete one full cycle — the whole point of a Strobe, since no
    /// bar that fits a terminal can plot a Deviation this small.
    #[test]
    fn half_a_cent_flat_on_g3_completes_one_cycle_in_about_18_seconds() {
        let target = 196.00_f32;
        let detected = target * 2f32.powf(-0.5 / 1200.0);
        let deviation_hz = (detected - target).abs();
        let period_secs = 1.0 / deviation_hz;
        assert!(
            (16.0..20.0).contains(&period_secs),
            "expected a ~18s period, got {period_secs:.2}s"
        );

        let mut s = Strobe::new();
        let dt = 0.021; // ~ HOP_MS
        let mut elapsed = 0.0;
        while elapsed < period_secs {
            s.advance(detected, target, dt);
            elapsed += dt;
        }
        // A completed cycle lands back near phase 0 (measuring the shorter way around the
        // wrap), not stuck partway (which would mean the rate was wrong) or torn arbitrarily far
        // from 0 (which would mean it never actually cycled).
        let wrapped_distance = s.phase().min(TAU - s.phase());
        assert!(
            wrapped_distance < 0.3,
            "phase did not complete a cycle after {elapsed:.1}s, got {:.3} rad",
            s.phase()
        );
    }

    #[test]
    fn a_long_stall_jumps_the_phase_without_panicking_or_producing_nan() {
        let mut s = Strobe::new();
        s.advance(196.05, 196.0, 1000.0); // a huge dt: the render loop stalled for a long time
        assert!(s.phase().is_finite());
        assert!((0.0..TAU).contains(&s.phase()));
    }

    #[test]
    fn phase_always_stays_within_zero_to_tau() {
        let mut s = Strobe::new();
        for (f_detected, dt) in [(196.5, 0.021), (195.0, 5.0), (300.0, 0.5), (50.0, 2.0)] {
            s.advance(f_detected, 196.0, dt);
            assert!(
                (0.0..TAU).contains(&s.phase()),
                "phase {} escaped [0, TAU)",
                s.phase()
            );
        }
    }
}
