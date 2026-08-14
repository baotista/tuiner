//! Turns a stream of samples drained from the audio-source queue into a stream of `Reading`s.
//! Shared by the live binary and by tests driving a corpus clip through `audio::WavSource`, so
//! both exercise the same analysis loop.

use rtrb::Consumer;

use crate::detect::{self, Reading};
use crate::{LOWPASS_MULT, MAX_HZ, MIN_HZ, hop_samples, window_samples};

/// Drains `consumer` until its `AudioSource` is dropped, running the detector once per hop and
/// calling `on_reading` for every window with a usable Pitch.
pub fn run(mut consumer: Consumer<f32>, sample_rate: u32, mut on_reading: impl FnMut(Reading)) {
    let window = window_samples(sample_rate);
    let hop = hop_samples(sample_rate).max(1);
    let mut detector = detect::Detector::new(sample_rate as f32, window, MIN_HZ, MAX_HZ)
        .with_lowpass(Some(LOWPASS_MULT));

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
                    if filled == window
                        && let Some(reading) = detector.analyse(&buf)
                    {
                        on_reading(reading);
                    }
                }
            }
            Err(_) if consumer.is_abandoned() => break,
            Err(_) => std::thread::yield_now(),
        }
    }
}
