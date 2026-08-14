//! The Chromatic Mode readout: capture the default Input Device, gate and smooth the sounding
//! Pitch, and render it — nearest Note, Hz, Deviation, and a coarse ±50 cent bar.
//!
//! Two threads and one lock-free queue, per the PRD's architecture: the realtime `cpal`
//! callback does one job — deinterleave into the queue — and this thread polls keyboard input,
//! drains samples, runs the pipeline, and renders, all in the same loop. Coupling analysis to
//! the render loop is affordable (well under 1% of a core) and keeps the failure mode benign: a
//! stall here just means a stale reading once it catches up.

use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use rtrb::RingBuffer;
use tuiner::audio::{AudioSource, LiveCapture};
use tuiner::pipeline::{Frame, Pipeline, Polled};
use tuiner::ui::{self, Readout};
use tuiner::{pitch, window_samples};

fn main() {
    let source = LiveCapture::default_device().expect("no default Input Device available");
    let sample_rate = source.sample_rate();
    let window = window_samples(sample_rate);

    let (producer, consumer) = RingBuffer::<f32>::new(window * 4);
    let _handle = Box::new(source).start(producer);
    let mut pipeline = Pipeline::new(consumer, sample_rate);

    let mut terminal = ratatui::init();
    let mut readout = Readout::Listening;
    loop {
        if event::poll(Duration::from_millis(10)).unwrap_or(false)
            && let Ok(Event::Key(key)) = event::read()
            && key.kind == KeyEventKind::Press
            && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
        {
            break;
        }

        if let Polled::Frame(frame) = pipeline.poll() {
            readout = to_readout(frame);
        }

        terminal
            .draw(|f| ui::render(f, f.area(), &readout))
            .expect("failed to draw frame");
    }
    ratatui::restore();
}

fn to_readout(frame: Frame) -> Readout {
    let (hz, dimmed) = match frame {
        Frame::Pitched { hz, .. } => (hz, false),
        Frame::Unpitched => return Readout::Listening,
        Frame::Silent {
            held: Some((hz, _clarity)),
        } => (hz, true),
        Frame::Silent { held: None } => return Readout::Listening,
    };
    let (note, cents) = pitch::nearest_note(hz, pitch::DEFAULT_REFERENCE_PITCH);
    Readout::Reading {
        note,
        hz,
        cents,
        dimmed,
    }
}
