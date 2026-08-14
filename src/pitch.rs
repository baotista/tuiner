//! Note, Pitch, Deviation, and Reference Pitch — the twelve-tone equal temperament conversions
//! between them. Tiny interface, used everywhere, essentially frozen once written.

/// The frequency assigned to A4, from which every other Note's frequency is derived. A4 = 440 Hz
/// until Reference Pitch persistence lands.
pub const DEFAULT_REFERENCE_PITCH: f32 = 440.0;

/// The Deviation band inside which a String (or, in Chromatic Mode, a Note) counts as correctly
/// tuned.
pub const IN_TUNE_TOLERANCE_CENTS: f32 = 3.0;

const NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// The nearest Note to `hz` (e.g. "E2") and the Deviation from it in cents, at the given
/// Reference Pitch. Negative Deviation is flat, positive is sharp.
pub fn nearest_note(hz: f32, reference_pitch: f32) -> (String, f32) {
    let midi = 69.0 + 12.0 * (hz / reference_pitch).log2();
    let nearest = midi.round();
    let cents = (midi - nearest) * 100.0;
    let n = nearest as i32;
    // div_euclid, not `/`: plain integer division truncates toward zero, which is off by one
    // octave for any negative MIDI number (Rust's `/` gives -3/12 == 0, not the floor -1).
    let name = format!(
        "{}{}",
        NAMES[(n.rem_euclid(12)) as usize],
        n.div_euclid(12) - 1
    );
    (name, cents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b0_reads_correctly() {
        let (note, cents) = nearest_note(30.868, DEFAULT_REFERENCE_PITCH);
        assert_eq!(note, "B0");
        assert!(
            cents.abs() < 1.0,
            "B0 deviation {cents:+.2}c, expected near 0"
        );
    }

    #[test]
    fn e6_reads_correctly() {
        let (note, cents) = nearest_note(1318.51, DEFAULT_REFERENCE_PITCH);
        assert_eq!(note, "E6");
        assert!(
            cents.abs() < 1.0,
            "E6 deviation {cents:+.2}c, expected near 0"
        );
    }

    #[test]
    fn negative_deviation_is_flat() {
        // A hair below E2 (82.407 Hz) must read negative, per the fixed sign convention.
        let (_note, cents) = nearest_note(82.0, DEFAULT_REFERENCE_PITCH);
        assert!(
            cents < 0.0,
            "expected a flat (negative) deviation, got {cents:+.2}c"
        );
    }

    #[test]
    fn positive_deviation_is_sharp() {
        let (_note, cents) = nearest_note(83.0, DEFAULT_REFERENCE_PITCH);
        assert!(
            cents > 0.0,
            "expected a sharp (positive) deviation, got {cents:+.2}c"
        );
    }

    /// Regression: a negative MIDI number (reachable with an unusual Reference Pitch) used to
    /// floor toward zero instead of down, reading one octave too high.
    #[test]
    fn negative_midi_numbers_floor_the_octave_correctly() {
        let (note, _cents) = nearest_note(30.868, 2000.0);
        assert_eq!(note, "A-2", "octave floored toward zero instead of down");
    }
}
