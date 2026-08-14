//! Turns a stream of samples drained from the audio-source queue into a stream of gated,
//! smoothed `Frame`s. Shared by the live binary and by tests driving a corpus clip through
//! `audio::WavSource`, so both exercise the same analysis loop.

use rtrb::Consumer;

use crate::gate::{self, NoiseFloor, State};
use crate::smoothing::Smoother;
use crate::{HOP_MS, LOWPASS_MULT, MAX_HZ, MIN_HZ, hop_samples, window_samples};

/// What one hop of audio produced, after gating and smoothing.
#[derive(Debug, Clone, Copy)]
pub enum Frame {
    /// A trusted Pitch: Level and Clarity both passed, and it has been median+EMA smoothed.
    Pitched {
        hz: f32,
        clarity: f32,
        k_used: usize,
    },
    /// Level ok but not periodic enough to trust — no reading at all.
    Unpitched,
    /// Level below the gate. Carries the last Pitched reading, if any, to hold and dim.
    Silent { held: Option<(f32, f32)> },
}

/// Drains `consumer` until its `AudioSource` is dropped, running the detector once per hop,
/// gating the result, and calling `on_frame` with what that hop produced.
pub fn run(mut consumer: Consumer<f32>, sample_rate: u32, mut on_frame: impl FnMut(Frame)) {
    let window = window_samples(sample_rate);
    let hop = hop_samples(sample_rate).max(1);
    let mut detector = crate::detect::Detector::new(sample_rate as f32, window, MIN_HZ, MAX_HZ)
        .with_lowpass(Some(LOWPASS_MULT));
    let fps = 1000.0 / HOP_MS;
    let mut floor = NoiseFloor::new(fps);
    let mut smoother = Smoother::new();
    let mut held: Option<(f32, f32)> = None;

    let mut buf = vec![0.0f32; window];
    let mut filled = 0usize;
    let mut hop_buf = Vec::with_capacity(hop);

    loop {
        match consumer.pop() {
            Ok(sample) => {
                hop_buf.push(sample);
                if hop_buf.len() == hop {
                    buf.copy_within(hop.., 0);
                    buf[window - hop..].copy_from_slice(&hop_buf);
                    hop_buf.clear();
                    filled = (filled + hop).min(window);
                    if filled == window {
                        let level_db = gate::level_db(&buf);
                        floor.observe(level_db);

                        let reading = detector.analyse(&buf);
                        let clarity = reading.as_ref().map(|r| r.clarity).unwrap_or(0.0);
                        let state = gate::classify(level_db, floor.gate_db(), clarity);

                        match state {
                            State::Pitched => {
                                let r = reading.expect("Pitched implies a reading");
                                let hz = smoother.push(r.refined_hz);
                                held = Some((hz, r.clarity));
                                on_frame(Frame::Pitched {
                                    hz,
                                    clarity: r.clarity,
                                    k_used: r.k_used,
                                });
                            }
                            State::Unpitched => {
                                // Deliberately not reset here: a single Unpitched hop is often
                                // a momentary Clarity dip mid-note (~8.7% of real note frames
                                // fall below 0.90, ADR 0001), not the note actually stopping.
                                // Resetting would drop the Smoother's history right before the
                                // next Pitched hop, so that hop would pass through un-median'd
                                // and un-eased — defeating median-3 at exactly the isolated-
                                // frame case it exists to catch. Silent, where Level itself
                                // dropped, is the real "the note stopped" signal.
                                on_frame(Frame::Unpitched);
                            }
                            State::Silent => {
                                smoother.reset();
                                on_frame(Frame::Silent { held });
                            }
                        }
                    }
                }
            }
            Err(_) if consumer.is_abandoned() => break,
            Err(_) => std::thread::yield_now(),
        }
    }
}
