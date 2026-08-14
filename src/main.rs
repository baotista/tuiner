//! Tracer bullet: capture the default Input Device, detect the sounding Pitch, print its
//! nearest Note. Display is crude on purpose — the "Chromatic Mode readout" slice replaces it.

use rtrb::RingBuffer;
use tuiner::audio::{AudioSource, LiveCapture};
use tuiner::{detect, pipeline, window_samples};

fn main() {
    let source = LiveCapture::default_device().expect("no default Input Device available");
    let sample_rate = source.sample_rate();
    let window = window_samples(sample_rate);

    let (producer, consumer) = RingBuffer::<f32>::new(window * 4);
    let _handle = Box::new(source).start(producer);

    println!("listening at {sample_rate} Hz — Ctrl+C to quit");
    pipeline::run(consumer, sample_rate, |reading| {
        let (note, cents) = detect::nearest_note(reading.refined_hz);
        println!(
            "{note:<4} {cents:>+6.1}c  {:>8.2} Hz  clarity {:.2}",
            reading.refined_hz, reading.clarity
        );
    });
}
