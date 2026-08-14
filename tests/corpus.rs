//! Drives the whole pipeline — `WavSource` through the queue through `detect` — from each
//! committed corpus clip, with no audio hardware and no terminal. Exercises the audio-source
//! seam end to end and, incidentally, the 44.1 kHz corpus against the 48 kHz-tuned constants.
//! Mirrors the PRD's "End-to-end through the seam" testing decision: what each clip pins is
//! documented in `corpus/README.md`.

use rtrb::RingBuffer;
use tuiner::audio::{AudioSource, WavSource};
use tuiner::detect::Reading;
use tuiner::{detect, pipeline, window_samples};

fn readings_from_clip(path: &str) -> Vec<Reading> {
    let source = WavSource::open(path).expect("corpus clip must be present");
    let sample_rate = source.sample_rate();
    let window = window_samples(sample_rate);

    let (producer, consumer) = RingBuffer::<f32>::new(window * 4);
    let _handle = Box::new(source).start(producer);

    let mut readings = Vec::new();
    pipeline::run(consumer, sample_rate, |reading| readings.push(reading));
    readings
}

#[test]
fn bass_low_e_tracks_e1_e2_with_no_octave_errors() {
    // Clarity 0.90 is the threshold ADR 0001 pinned from real-audio measurement — silence and
    // chords fall below it, isolated note frames above it.
    let notes: Vec<String> = readings_from_clip("corpus/bass-low-e.wav")
        .into_iter()
        .filter(|r| r.clarity >= 0.90)
        .map(|r| detect::nearest_note(r.refined_hz).0)
        .collect();

    assert!(
        !notes.is_empty(),
        "no Pitched frames at all from the corpus clip"
    );
    for note in &notes {
        assert!(
            note == "E1" || note == "E2",
            "bass low E clip read {note}, expected only E1 or E2 — an octave error"
        );
    }
}

#[test]
fn hum_produces_almost_no_high_clarity_frames() {
    // hum.wav is a guitar plugged in but not played — corpus/README.md pins its Clarity
    // ceiling at 0.83, below the 0.90 threshold, so it should admit essentially nothing.
    let readings = readings_from_clip("corpus/hum.wav");
    let passing = readings.iter().filter(|r| r.clarity >= 0.90).count();
    assert!(
        passing == 0,
        "hum clip had {passing}/{} frames at clarity>=0.90, expected none",
        readings.len()
    );
}

#[test]
fn chord_admits_only_a_small_minority_of_frames() {
    // corpus/README.md pins chord admission at 4.1% at the 0.90 threshold — Clarity alone
    // cannot reject a chord outright, only keep it to a small minority.
    let readings = readings_from_clip("corpus/chord.wav");
    assert!(
        !readings.is_empty(),
        "expected some frames from the chord clip"
    );
    let passing = readings.iter().filter(|r| r.clarity >= 0.90).count();
    let fraction = passing as f32 / readings.len() as f32;
    assert!(
        fraction < 0.15,
        "chord clip admitted {:.1}% of frames at clarity>=0.90, expected a small minority",
        fraction * 100.0
    );
}

#[test]
fn guitar_top_e_holds_k_through_decay() {
    // corpus/README.md pins this clip to one claim: k does not degrade as the pluck decays.
    let readings = readings_from_clip("corpus/gtr-top-e.wav");
    let accepted: Vec<&Reading> = readings.iter().filter(|r| r.clarity >= 0.90).collect();
    assert!(
        !accepted.is_empty(),
        "expected some high-clarity frames from the guitar clip"
    );
    let tail_start = accepted.len() * 3 / 4;
    for r in &accepted[tail_start..] {
        assert!(
            r.k_used > 1,
            "k collapsed to {} in the decayed tail of the guitar clip",
            r.k_used
        );
    }
}
