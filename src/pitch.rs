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
    (note_name(nearest as i32), cents)
}

/// The Note name for a MIDI note number (e.g. `69` -> "A4"). Exists alongside [`nearest_note`]
/// so a Tuning's fixed Target Pitches can be labelled from the same numbering, rather than a
/// hand-typed name risking disagreement with the Hz [`midi_to_hz`] computes for it.
pub fn note_name(midi: i32) -> String {
    // div_euclid, not `/`: plain integer division truncates toward zero, which is off by one
    // octave for any negative MIDI number (Rust's `/` gives -3/12 == 0, not the floor -1).
    format!(
        "{}{}",
        NAMES[(midi.rem_euclid(12)) as usize],
        midi.div_euclid(12) - 1
    )
}

/// The frequency of a MIDI note number at the given Reference Pitch — the inverse of the pitch
/// half of [`nearest_note`]. A Tuning's Target Pitches are derived from this, not hand-typed, so
/// they can never drift from the equal-temperament relationship the rest of the app assumes.
pub fn midi_to_hz(midi: i32, reference_pitch: f32) -> f32 {
    reference_pitch * 2f32.powf((midi - 69) as f32 / 12.0)
}

/// The Target Pitch a detected `hz` is `cents` away from — the inverse of the Deviation half of
/// `nearest_note`. Exists for the Strobe's phase accumulator, which needs a Target Pitch in
/// hertz rather than a Deviation in cents.
pub fn target_hz(hz: f32, cents: f32) -> f32 {
    hz / 2f32.powf(cents / 1200.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_hz_inverts_nearest_notes_deviation() {
        let hz = 195.5;
        let (_note, cents) = nearest_note(hz, DEFAULT_REFERENCE_PITCH);
        let target = target_hz(hz, cents);
        let (_note2, roundtrip_cents) = nearest_note(target, DEFAULT_REFERENCE_PITCH);
        assert!(
            roundtrip_cents.abs() < 0.01,
            "expected the Target Pitch to read as ~0 Deviation from itself, got {roundtrip_cents}"
        );
    }

    #[test]
    fn midi_to_hz_and_note_name_round_trip_through_nearest_note() {
        // A4 is midi 69 by definition — the anchor the whole conversion is built from.
        assert_eq!(note_name(69), "A4");
        assert!((midi_to_hz(69, DEFAULT_REFERENCE_PITCH) - 440.0).abs() < 0.01);

        for midi in [28, 40, 55, 62, 69, 76] {
            let hz = midi_to_hz(midi, DEFAULT_REFERENCE_PITCH);
            let (note, cents) = nearest_note(hz, DEFAULT_REFERENCE_PITCH);
            assert_eq!(note, note_name(midi));
            assert!(cents.abs() < 0.01, "expected ~0c for {note}, got {cents}");
        }
    }

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
