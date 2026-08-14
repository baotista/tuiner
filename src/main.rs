//! Tracer bullet: capture the default Input Device, detect the sounding Pitch, print its
//! nearest Note. Display is crude on purpose — the "Chromatic Mode readout" slice replaces it.

use rtrb::RingBuffer;
use tuiner::audio::{AudioSource, LiveCapture};
use tuiner::pipeline::{self, Frame};
use tuiner::{detect, window_samples};

fn main() {
    let source = LiveCapture::default_device().expect("no default Input Device available");
    let sample_rate = source.sample_rate();
    let window = window_samples(sample_rate);

    let (producer, consumer) = RingBuffer::<f32>::new(window * 4);
    let _handle = Box::new(source).start(producer);

    println!("listening at {sample_rate} Hz — Ctrl+C to quit");
    pipeline::run(consumer, sample_rate, |frame| match frame {
        Frame::Pitched { hz, clarity, .. } => {
            let (note, cents) = detect::nearest_note(hz);
            println!("{note:<4} {cents:>+6.1}c  {hz:>8.2} Hz  clarity {clarity:.2}");
        }
        Frame::Unpitched => println!("listening..."),
        Frame::Silent {
            held: Some((hz, clarity)),
        } => {
            let (note, cents) = detect::nearest_note(hz);
            println!("\x1b[2m{note:<4} {cents:>+6.1}c  {hz:>8.2} Hz  clarity {clarity:.2}\x1b[0m");
        }
        Frame::Silent { held: None } => println!("listening..."),
    });
}
