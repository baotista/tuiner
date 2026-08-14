//! Drives the whole pipeline — `WavSource` through the queue through `detect` through `gate` —
//! from each committed corpus clip, with no audio hardware and no terminal. Mirrors the PRD's
//! "End-to-end through the seam" testing decision: what each clip pins is documented in
//! `corpus/README.md`.

use rtrb::RingBuffer;
use tuiner::audio::{AudioSource, WavSource};
use tuiner::detect::Reading;
use tuiner::pipeline::{Frame, Pipeline};
use tuiner::{LOWPASS_MULT, MAX_HZ, MIN_HZ, detect, hop_samples, pitch, tuning, window_samples};

fn frames_from_clip(path: &str) -> Vec<Frame> {
    let source = WavSource::open(path).expect("corpus clip must be present");
    let sample_rate = source.sample_rate();
    let window = window_samples(sample_rate);

    let (producer, consumer) = RingBuffer::<f32>::new(window * 4);
    let _handle = Box::new(source).start(producer);

    let mut pipeline = Pipeline::new(consumer, sample_rate);
    let mut frames = Vec::new();
    loop {
        let ended = pipeline.drain(|frame| frames.push(frame));
        if ended {
            break;
        }
        std::thread::yield_now();
    }
    frames
}

/// Every raw `Reading` from the detector, bypassing `gate` and `smoothing` entirely. Median-3
/// absorbs a single-hop octave error before it reaches a `Frame`, so an octave-stability
/// regression needs a path that sees the detector's own output directly to be caught.
fn raw_readings_from_clip(path: &str) -> Vec<Reading> {
    let source = WavSource::open(path).expect("corpus clip must be present");
    let sample_rate = source.sample_rate();
    let window = window_samples(sample_rate);
    let hop = hop_samples(sample_rate).max(1);

    let (producer, mut consumer) = RingBuffer::<f32>::new(window * 4);
    let _handle = Box::new(source).start(producer);

    let mut detector = detect::Detector::new(sample_rate as f32, window, MIN_HZ, MAX_HZ)
        .with_lowpass(Some(LOWPASS_MULT));
    let mut buf = vec![0.0f32; window];
    let mut filled = 0usize;
    let mut hop_buf = Vec::with_capacity(hop);
    let mut readings = Vec::new();

    loop {
        match consumer.pop() {
            Ok(sample) => {
                hop_buf.push(sample);
                if hop_buf.len() == hop {
                    buf.copy_within(hop.., 0);
                    buf[window - hop..].copy_from_slice(&hop_buf);
                    hop_buf.clear();
                    filled = (filled + hop).min(window);
                    if filled == window
                        && let Some(reading) = detector.analyse(&buf)
                    {
                        readings.push(reading);
                    }
                }
            }
            Err(_) if consumer.is_abandoned() => break,
            Err(_) => std::thread::yield_now(),
        }
    }
    readings
}

fn pitched_fraction(frames: &[Frame]) -> f32 {
    let pitched = frames
        .iter()
        .filter(|f| matches!(f, Frame::Pitched { .. }))
        .count();
    pitched as f32 / frames.len() as f32
}

#[test]
fn hum_produces_zero_pitched_frames() {
    let frames = frames_from_clip("corpus/hum.wav");
    let pitched = frames
        .iter()
        .filter(|f| matches!(f, Frame::Pitched { .. }))
        .count();
    assert_eq!(
        pitched, 0,
        "hum clip (guitar plugged in, not played) produced {pitched} Pitched frames, expected zero"
    );
}

#[test]
fn chord_produces_fewer_than_5_percent_pitched_frames() {
    let frames = frames_from_clip("corpus/chord.wav");
    assert!(
        !frames.is_empty(),
        "expected some frames from the chord clip"
    );
    let fraction = pitched_fraction(&frames);
    assert!(
        fraction < 0.05,
        "chord clip: {:.1}% Pitched, expected fewer than 5%",
        fraction * 100.0
    );
}

#[test]
fn bass_low_e_produces_a_large_majority_of_pitched_frames_with_no_octave_errors() {
    let frames = frames_from_clip("corpus/bass-low-e.wav");
    assert!(
        !frames.is_empty(),
        "expected some frames from the bass low E clip"
    );
    let fraction = pitched_fraction(&frames);
    assert!(
        fraction > 0.70,
        "bass clip: only {:.1}% Pitched, expected a large majority",
        fraction * 100.0
    );

    for frame in &frames {
        if let Frame::Pitched { hz, .. } = frame {
            let (note, _cents) = pitch::nearest_note(*hz, pitch::DEFAULT_REFERENCE_PITCH);
            assert!(
                note == "E1" || note == "E2",
                "bass low E clip read {note}, expected only E1 or E2 — an octave error"
            );
        }
    }
}

/// Issue #8's stronger claim than the no-octave-errors check above: not just that every Pitched
/// frame reads E1 or E2, but that the clip's alternation actually crosses that boundary about ten
/// times — the corpus README's own description of the clip, and the specific place an octave
/// error would surface as a spurious extra crossing or a missed one.
#[test]
fn bass_low_e_crosses_the_e1_e2_boundary_about_ten_times() {
    let frames = frames_from_clip("corpus/bass-low-e.wav");
    let notes: Vec<String> = frames
        .iter()
        .filter_map(|f| match f {
            Frame::Pitched { hz, .. } => {
                Some(pitch::nearest_note(*hz, pitch::DEFAULT_REFERENCE_PITCH).0)
            }
            _ => None,
        })
        .collect();
    assert!(!notes.is_empty(), "expected some Pitched frames to track");

    let crossings = notes.windows(2).filter(|w| w[0] != w[1]).count();
    assert!(
        (8..=12).contains(&crossings),
        "expected roughly ten E1/E2 crossings, got {crossings}"
    );
}

/// The Chromatic-level check above pins the detector's own octave stability, but issue #8 is
/// about Guided Mode specifically: every E1 frame must also match Bass Standard's String 4 (E1)
/// through the real `Tuning::match_pitch` entry point, with no octave confusion introduced by
/// Capture Range matching itself.
#[test]
fn bass_low_e_frames_match_bass_standards_e1_string_through_guided_mode() {
    let frames = frames_from_clip("corpus/bass-low-e.wav");
    let bass = tuning::all()
        .into_iter()
        .find(|t| t.name == "Bass Standard")
        .unwrap();

    let mut e1_frames_checked = 0;
    for frame in &frames {
        if let Frame::Pitched { hz, .. } = frame {
            let (note, _cents) = pitch::nearest_note(*hz, pitch::DEFAULT_REFERENCE_PITCH);
            if note != "E1" {
                continue;
            }
            e1_frames_checked += 1;
            match bass.match_pitch(*hz) {
                tuning::Match::String { number, note, .. } => {
                    assert_eq!(number, 4, "E1 matched String {number} ({note}) instead");
                    assert_eq!(note, "E1");
                }
                tuning::Match::Note { note, .. } => {
                    panic!("E1 fell back to Note {note} instead of matching String 4")
                }
            }
        }
    }
    assert!(
        e1_frames_checked > 0,
        "expected some E1 frames to check Guided Mode matching against"
    );
}

/// Regression: `frames_from_clip`'s octave check above only ever sees the median-3-smoothed
/// output, which by design absorbs a single isolated bad hop — exactly the kind of one-hop
/// octave error the median exists to hide. Check the detector's raw readings directly too, the
/// same way ADR 0001's original prototype validation did.
#[test]
fn bass_low_e_raw_readings_show_no_octave_errors_at_clarity_090() {
    let readings = raw_readings_from_clip("corpus/bass-low-e.wav");
    assert!(
        !readings.is_empty(),
        "expected some raw readings from the bass low E clip"
    );
    for r in readings.iter().filter(|r| r.clarity >= 0.90) {
        let (note, _cents) = pitch::nearest_note(r.refined_hz, pitch::DEFAULT_REFERENCE_PITCH);
        assert!(
            note == "E1" || note == "E2",
            "raw reading at clarity {:.2} read {note}, expected only E1 or E2 — an octave error",
            r.clarity
        );
    }
}

#[test]
fn guitar_top_e_produces_a_large_majority_of_pitched_frames_and_holds_k_through_decay() {
    let frames = frames_from_clip("corpus/gtr-top-e.wav");
    assert!(
        !frames.is_empty(),
        "expected some frames from the guitar top E clip"
    );
    let fraction = pitched_fraction(&frames);
    assert!(
        fraction > 0.70,
        "guitar clip: only {:.1}% Pitched, expected a large majority",
        fraction * 100.0
    );

    let pitched: Vec<&Frame> = frames
        .iter()
        .filter(|f| matches!(f, Frame::Pitched { .. }))
        .collect();
    let tail_start = pitched.len() * 3 / 4;
    for frame in &pitched[tail_start..] {
        let Frame::Pitched { k_used, .. } = frame else {
            unreachable!()
        };
        assert!(
            *k_used > 1,
            "k collapsed to {k_used} in the decayed tail of the guitar clip"
        );
    }
}
