//! Turns a stream of samples drained from the audio-source queue into gated, smoothed `Frame`s.
//!
//! `Pipeline::poll` never blocks: the caller drives it, one call per iteration of its own loop
//! (interleaved with keyboard input and rendering), matching the PRD's two-thread architecture
//! — the realtime `cpal` callback does one job, and the main thread polls keyboard input,
//! drains samples, runs the pipeline, and renders, all in the same loop. Analysis costs well
//! under 1% of a core, so coupling it to the render loop is affordable, and it keeps the
//! documented failure mode benign: a caller that stalls just falls behind and sees a stale
//! reading once it catches up, rather than a second thread going silently dark.

use rtrb::Consumer;

use crate::detect::Detector;
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

/// What one `Pipeline::poll` call found.
#[derive(Debug, Clone, Copy)]
pub enum Polled {
    /// No hop has completed since the last call.
    Pending,
    /// A hop completed. If the caller fell behind and several hops completed in one call, this
    /// is only the most recent — falling behind should produce a stale reading, not a backlog
    /// to work through.
    Frame(Frame),
    /// The audio source has stopped: a live stream ended, or a replayed clip finished.
    Ended,
}

/// Owns the analysis state for one audio-source queue: the detector, the tracked noise floor,
/// the smoother, and the sliding analysis window.
pub struct Pipeline {
    consumer: Consumer<f32>,
    detector: Detector,
    floor: NoiseFloor,
    smoother: Smoother,
    held: Option<(f32, f32)>,
    buf: Vec<f32>,
    filled: usize,
    hop_buf: Vec<f32>,
    hop: usize,
    window: usize,
}

impl Pipeline {
    pub fn new(consumer: Consumer<f32>, sample_rate: u32) -> Self {
        let window = window_samples(sample_rate);
        let hop = hop_samples(sample_rate).max(1);
        let detector = Detector::new(sample_rate as f32, window, MIN_HZ, MAX_HZ)
            .with_lowpass(Some(LOWPASS_MULT));
        let fps = 1000.0 / HOP_MS;
        Self {
            consumer,
            detector,
            floor: NoiseFloor::new(fps),
            smoother: Smoother::new(),
            held: None,
            buf: vec![0.0; window],
            filled: 0,
            hop_buf: Vec::with_capacity(hop),
            hop,
            window,
        }
    }

    /// Non-blocking: drains whatever is queued right now and reports only the most recent
    /// Frame, for a caller — the render loop — that only cares about the latest reading and
    /// treats falling behind as benign, per the documented failure mode.
    pub fn poll(&mut self) -> Polled {
        let mut latest = None;
        let ended = self.drain(|frame| latest = Some(frame));
        match (latest, ended) {
            (Some(frame), _) => Polled::Frame(frame),
            (None, true) => Polled::Ended,
            (None, false) => Polled::Pending,
        }
    }

    /// Drains whatever samples are currently queued, calling `on_frame` for every hop that
    /// completes, in order — unlike `poll`, nothing is discarded. For driving a recorded clip
    /// end to end (tests, offline analysis), where every classification matters and not just
    /// the most recent: a `WavSource` replays at memory speed with no real-time pacing, so the
    /// queue can hold an entire clip's worth of hops between two calls, and `poll`'s "keep only
    /// the latest" design would silently collapse all of them into one.
    ///
    /// Returns whether the source has now ended (abandoned, and drained dry).
    pub fn drain(&mut self, mut on_frame: impl FnMut(Frame)) -> bool {
        loop {
            match self.consumer.pop() {
                Ok(sample) => {
                    self.hop_buf.push(sample);
                    if self.hop_buf.len() == self.hop {
                        self.buf.copy_within(self.hop.., 0);
                        let (window, hop) = (self.window, self.hop);
                        self.buf[window - hop..].copy_from_slice(&self.hop_buf);
                        self.hop_buf.clear();
                        self.filled = (self.filled + hop).min(window);
                        if self.filled == window {
                            on_frame(self.process_hop());
                        }
                    }
                }
                Err(_) if self.consumer.is_abandoned() => return true,
                Err(_) => return false,
            }
        }
    }

    fn process_hop(&mut self) -> Frame {
        let level_db = gate::level_db(&self.buf);
        self.floor.observe(level_db);

        let reading = self.detector.analyse(&self.buf);
        let clarity = reading.as_ref().map(|r| r.clarity).unwrap_or(0.0);
        let state = gate::classify(level_db, self.floor.gate_db(), clarity);

        match state {
            State::Pitched => {
                let r = reading.expect("Pitched implies a reading");
                let hz = self.smoother.push(r.refined_hz);
                self.held = Some((hz, r.clarity));
                Frame::Pitched {
                    hz,
                    clarity: r.clarity,
                    k_used: r.k_used,
                }
            }
            State::Unpitched => {
                // Deliberately not reset here: a single Unpitched hop is often a momentary
                // Clarity dip mid-note (~8.7% of real note frames fall below 0.90, ADR 0001),
                // not the note actually stopping. Resetting would drop the Smoother's history
                // right before the next Pitched hop, so that hop would pass through un-
                // median'd and un-eased — defeating median-3 at exactly the isolated-frame
                // case it exists to catch. Silent, where Level itself dropped, is the real
                // "the note stopped" signal.
                Frame::Unpitched
            }
            State::Silent => {
                self.smoother.reset();
                Frame::Silent { held: self.held }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtrb::RingBuffer;

    /// A `WavSource` replays at memory speed with no real-time pacing, so it's easy for a
    /// producer to push several hops' worth of samples before the consumer ever calls in —
    /// exactly what broke `poll`'s original design when it was reused for exhaustive test
    /// replay. Feed two pipelines the identical sequence and confirm `poll` really does report
    /// only one Frame while `drain` reports every completed hop.
    #[test]
    fn poll_reports_only_the_latest_frame_but_drain_reports_every_one() {
        let sample_rate = 8_000u32;
        let window = window_samples(sample_rate);
        let hop = hop_samples(sample_rate).max(1);
        let hops_to_push = window / hop + 5;

        let (mut tx_a, rx_a) = RingBuffer::<f32>::new(window * 8);
        let (mut tx_b, rx_b) = RingBuffer::<f32>::new(window * 8);
        for i in 0..(hops_to_push * hop) {
            let s = if i % 50 < 25 { 0.2 } else { -0.2 };
            tx_a.push(s).unwrap();
            tx_b.push(s).unwrap();
        }

        let mut poll_pipeline = Pipeline::new(rx_a, sample_rate);
        let polled = poll_pipeline.poll();
        assert!(
            matches!(polled, Polled::Frame(_)),
            "expected a Frame once the window filled, got {polled:?}"
        );

        let mut drain_pipeline = Pipeline::new(rx_b, sample_rate);
        let mut drained = Vec::new();
        drain_pipeline.drain(|f| drained.push(f));
        assert!(
            drained.len() > 1,
            "expected drain() to report every one of several completed hops, got {}",
            drained.len()
        );
    }
}
