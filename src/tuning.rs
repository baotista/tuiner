//! `Tuning`, `InstrumentString`, Target Pitch, derived Capture Range, and Guided Mode matching.
//! Ships five Tunings as data, not logic — Tuning is a flat concept, drawing no distinction
//! between open, modal and standard tunings, because that distinction matters for naming and not
//! at all for behaviour.
//!
//! A single matching entry point ([`Tuning::match_pitch`]) hides the neighbour-distance
//! derivation, the absolute cap, and the awkward cases (DADGAD's G3/A3 narrowing, the outermost
//! String on every Tuning hitting the cap) behind one call.

use crate::pitch;

/// Half the distance to the nearest neighbouring Target Pitch, minus this margin, is what a
/// String's Capture Range actually uses — so two Strings' ranges never touch, let alone overlap.
pub const NEIGHBOUR_MARGIN_CENTS: f32 = 15.0;

/// The absolute ceiling on a Capture Range. Exists because the outermost Strings have no
/// neighbour on one side — without it, the lowest String would claim any rumble beneath it.
pub const ABSOLUTE_CAP_CENTS: f32 = 250.0;

/// One of the instrument's playable strings, identified by its position — numbered from 1 at the
/// highest-pitched String upward, per CONTEXT.md.
#[derive(Debug, Clone, PartialEq)]
pub struct InstrumentString {
    pub number: u8,
    pub note: String,
    pub target_hz: f32,
    /// Derived from neighbouring Target Pitches at construction time, not configured.
    pub capture_range_cents: f32,
}

/// A named set of Target Pitches, one per String. A flat concept: nothing here distinguishes an
/// open, modal, or standard Tuning.
#[derive(Debug, Clone, PartialEq)]
pub struct Tuning {
    pub name: &'static str,
    pub strings: Vec<InstrumentString>,
}

/// What a sounding Pitch matched to, in Guided Mode.
#[derive(Debug, Clone, PartialEq)]
pub enum Match {
    /// Within a String's Capture Range: named by String and Deviation against its Target Pitch.
    String {
        number: u8,
        note: String,
        cents: f32,
    },
    /// Outside every String's Capture Range — refusing to guess, so the arrow never reverses
    /// direction while the player turns one Peg. Named against the nearest Note instead.
    Note { note: String, cents: f32 },
}

impl Tuning {
    /// Matches `hz` to the nearest String, but only reports it if `hz` actually falls within
    /// that String's own Capture Range — otherwise falls back to naming the nearest Note. Ranges
    /// never overlap by construction (see [`capture_ranges`]), so the String nearest by raw
    /// distance is always the one whose range would contain `hz`, if any range does.
    pub fn match_pitch(&self, hz: f32) -> Match {
        let nearest = self
            .strings
            .iter()
            .map(|s| (s, 1200.0 * (hz / s.target_hz).log2()))
            .min_by(|(_, a), (_, b)| a.abs().partial_cmp(&b.abs()).unwrap());

        match nearest {
            Some((s, cents)) if cents.abs() <= s.capture_range_cents => Match::String {
                number: s.number,
                note: s.note.clone(),
                cents,
            },
            _ => {
                let (note, cents) = pitch::nearest_note(hz, pitch::DEFAULT_REFERENCE_PITCH);
                Match::Note { note, cents }
            }
        }
    }
}

/// Derives a Capture Range for every position in `sorted_target_hz` (ascending order): half the
/// distance to the nearer neighbouring Target Pitch, minus [`NEIGHBOUR_MARGIN_CENTS`], capped at
/// [`ABSOLUTE_CAP_CENTS`]. The outermost positions have only one neighbour to measure against.
pub fn capture_ranges(sorted_target_hz: &[f32]) -> Vec<f32> {
    (0..sorted_target_hz.len())
        .map(|i| {
            let hz = sorted_target_hz[i];
            let mut nearest_gap_cents = f32::INFINITY;
            if i > 0 {
                nearest_gap_cents =
                    nearest_gap_cents.min(cents_between(sorted_target_hz[i - 1], hz));
            }
            if i + 1 < sorted_target_hz.len() {
                nearest_gap_cents =
                    nearest_gap_cents.min(cents_between(hz, sorted_target_hz[i + 1]));
            }
            (nearest_gap_cents / 2.0 - NEIGHBOUR_MARGIN_CENTS).clamp(0.0, ABSOLUTE_CAP_CENTS)
        })
        .collect()
}

fn cents_between(a_hz: f32, b_hz: f32) -> f32 {
    (1200.0 * (b_hz / a_hz).log2()).abs()
}

/// A String's number and MIDI note number, the raw material [`build`] turns into a full
/// [`InstrumentString`] — named so the tuple literals in [`all`] read as what they are rather
/// than two bare integers.
type StringDef = (u8, i32);

/// Builds a Tuning from `StringDef`s in any order — Capture Ranges are derived by Target Pitch,
/// not by the order given, and Strings are returned numbered 1..N for display regardless of
/// pitch order.
fn build(name: &'static str, strings: &[StringDef]) -> Tuning {
    let mut entries: Vec<(u8, i32, f32)> = strings
        .iter()
        .map(|&(number, midi)| {
            (
                number,
                midi,
                pitch::midi_to_hz(midi, pitch::DEFAULT_REFERENCE_PITCH),
            )
        })
        .collect();
    entries.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap());

    let sorted_hz: Vec<f32> = entries.iter().map(|e| e.2).collect();
    let ranges = capture_ranges(&sorted_hz);

    let mut strings: Vec<InstrumentString> = entries
        .into_iter()
        .zip(ranges)
        .map(
            |((number, midi, target_hz), capture_range_cents)| InstrumentString {
                number,
                note: pitch::note_name(midi),
                target_hz,
                capture_range_cents,
            },
        )
        .collect();
    strings.sort_by_key(|s| s.number);

    Tuning { name, strings }
}

/// The five Tunings the app ships, in the order `t` cycles through them.
pub fn all() -> Vec<Tuning> {
    vec![
        build(
            "Guitar Standard",
            &[(1, 64), (2, 59), (3, 55), (4, 50), (5, 45), (6, 40)],
        ),
        build(
            "Guitar D Standard",
            &[(1, 62), (2, 57), (3, 53), (4, 48), (5, 43), (6, 38)],
        ),
        build(
            "Guitar Open C",
            &[(1, 64), (2, 60), (3, 55), (4, 48), (5, 43), (6, 36)],
        ),
        build(
            "DADGAD",
            &[(1, 62), (2, 57), (3, 55), (4, 50), (5, 45), (6, 38)],
        ),
        build("Bass Standard", &[(1, 43), (2, 38), (3, 33), (4, 28)]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tuning(name: &str) -> Tuning {
        all().into_iter().find(|t| t.name == name).unwrap()
    }

    #[test]
    fn all_five_tunings_are_present_with_correct_target_pitches() {
        let tunings = all();
        assert_eq!(tunings.len(), 5);

        let by_name = |name: &str| tunings.iter().find(|t| t.name == name).unwrap();
        let notes = |t: &Tuning| -> Vec<String> {
            let mut s = t.strings.clone();
            s.sort_by_key(|s| s.number);
            s.iter().map(|s| s.note.clone()).collect()
        };

        assert_eq!(
            notes(by_name("Guitar Standard")),
            vec!["E4", "B3", "G3", "D3", "A2", "E2"]
        );
        assert_eq!(
            notes(by_name("Guitar D Standard")),
            vec!["D4", "A3", "F3", "C3", "G2", "D2"]
        );
        assert_eq!(
            notes(by_name("Guitar Open C")),
            vec!["E4", "C4", "G3", "C3", "G2", "C2"]
        );
        assert_eq!(
            notes(by_name("DADGAD")),
            vec!["D4", "A3", "G3", "D3", "A2", "D2"]
        );
        assert_eq!(
            notes(by_name("Bass Standard")),
            vec!["G2", "D2", "A1", "E1"]
        );
    }

    #[test]
    fn no_two_strings_in_any_tuning_can_both_claim_the_same_pitch() {
        // Sweep every whole cent across the full detector range and confirm at most one String
        // in each Tuning ever claims it.
        for tuning in all() {
            let mut hz = crate::MIN_HZ;
            while hz <= crate::MAX_HZ {
                let claimants = tuning
                    .strings
                    .iter()
                    .filter(|s| (1200.0 * (hz / s.target_hz).log2()).abs() <= s.capture_range_cents)
                    .count();
                assert!(
                    claimants <= 1,
                    "{} String{} at {hz:.2} Hz in Tuning {:?}",
                    claimants,
                    if claimants == 1 { "" } else { "s" },
                    tuning.name
                );
                hz *= 2f32.powf(1.0 / 1200.0); // step by one cent
            }
        }
    }

    #[test]
    fn dadgad_g3_and_a3_narrow_to_roughly_85_cents() {
        let dadgad = tuning("DADGAD");
        for note in ["G3", "A3"] {
            let s = dadgad.strings.iter().find(|s| s.note == note).unwrap();
            assert!(
                (80.0..=90.0).contains(&s.capture_range_cents),
                "{note} Capture Range was {:.1}c, expected roughly 85c",
                s.capture_range_cents
            );
        }
    }

    #[test]
    fn the_lowest_string_of_open_c_and_dadgad_hits_the_absolute_cap() {
        for tuning_name in ["Guitar Open C", "DADGAD"] {
            let t = tuning(tuning_name);
            let lowest = t
                .strings
                .iter()
                .min_by(|a, b| a.target_hz.partial_cmp(&b.target_hz).unwrap())
                .unwrap();
            assert_eq!(
                lowest.capture_range_cents, ABSOLUTE_CAP_CENTS,
                "{tuning_name}'s lowest String ({}) did not hit the absolute cap",
                lowest.note
            );
        }
    }

    #[test]
    fn a_bass_low_e_does_not_claim_a_sub_40hz_rumble() {
        let bass = tuning("Bass Standard");
        match bass.match_pitch(20.0) {
            Match::String { note, .. } => panic!("20 Hz rumble was claimed as {note}"),
            Match::Note { .. } => {}
        }
    }

    #[test]
    fn a_pitch_outside_every_capture_range_falls_back_to_the_nearest_note() {
        let dadgad = tuning("DADGAD");
        // Two semitones flat of G3 (whose Capture Range narrows to ~85c in DADGAD) lands well
        // outside every String's range — exactly the case that must refuse to guess rather than
        // guessing wrong while the player is still turning the Peg.
        let g3_hz = pitch::midi_to_hz(55, pitch::DEFAULT_REFERENCE_PITCH);
        let two_semitones_flat_of_g3 = g3_hz / 2f32.powf(2.0 / 12.0);
        match dadgad.match_pitch(two_semitones_flat_of_g3) {
            Match::String { note, cents, .. } => {
                panic!("expected a Note fallback, got String {note} at {cents:+.1}c")
            }
            Match::Note { note, .. } => assert_eq!(note, "F3"),
        }
    }

    #[test]
    fn strings_are_identified_correctly_regardless_of_play_order() {
        let standard = tuning("Guitar Standard");
        // Play the high E4 (String 1) exactly, then the low E2 (String 6) exactly — reverse
        // pitch order from how the Tuning was defined — and confirm matching doesn't care.
        let e4_hz = pitch::midi_to_hz(64, pitch::DEFAULT_REFERENCE_PITCH);
        let e2_hz = pitch::midi_to_hz(40, pitch::DEFAULT_REFERENCE_PITCH);

        match standard.match_pitch(e4_hz) {
            Match::String {
                number,
                note,
                cents,
            } => {
                assert_eq!(number, 1);
                assert_eq!(note, "E4");
                assert!(cents.abs() < 0.01);
            }
            Match::Note { note, .. } => panic!("expected String 1 (E4), got Note {note}"),
        }

        match standard.match_pitch(e2_hz) {
            Match::String {
                number,
                note,
                cents,
            } => {
                assert_eq!(number, 6);
                assert_eq!(note, "E2");
                assert!(cents.abs() < 0.01);
            }
            Match::Note { note, .. } => panic!("expected String 6 (E2), got Note {note}"),
        }
    }
}
