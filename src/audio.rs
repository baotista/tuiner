//! The audio-source seam: live capture and a WAV reader that replays a recorded clip, behind one
//! interface. This is a testability decision, not a generality one — it lets every stage from
//! `detect` onward run against the committed `corpus/` deterministically, with no hardware.

use std::path::Path;
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use rtrb::Producer;

/// Feeds one Input Channel's samples into a lock-free queue.
pub trait AudioSource {
    fn sample_rate(&self) -> u32;

    /// Starts feeding samples into `producer`. The returned handle must be kept alive for as
    /// long as samples should keep flowing — dropping it stops the source.
    fn start(self: Box<Self>, producer: Producer<f32>) -> SourceHandle;
}

/// Keeps a started `AudioSource` alive.
pub enum SourceHandle {
    Live(cpal::Stream),
    Replay(JoinHandle<()>),
}

/// The system's default Input Device, captured in realtime.
pub struct LiveCapture {
    device: cpal::Device,
    config: StreamConfig,
    sample_format: SampleFormat,
    channels: u16,
}

impl LiveCapture {
    pub fn default_device() -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or("no default Input Device")?;
        let supported = device.default_input_config().map_err(|e| e.to_string())?;
        Ok(Self {
            device,
            config: supported.config(),
            sample_format: supported.sample_format(),
            channels: supported.channels(),
        })
    }
}

impl AudioSource for LiveCapture {
    fn sample_rate(&self) -> u32 {
        self.config.sample_rate
    }

    fn start(self: Box<Self>, mut producer: Producer<f32>) -> SourceHandle {
        let channels = self.channels as usize;
        // The realtime callback's one job: deinterleave Input Channel 0 into the queue. No
        // allocation, no locking, no analysis — those happen on the consumer side.
        let stream = match self.sample_format {
            SampleFormat::F32 => self.device.build_input_stream(
                self.config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    for frame in data.chunks_exact(channels) {
                        let _ = producer.push(frame[0]);
                    }
                },
                |err| eprintln!("audio stream error: {err}"),
                None,
            ),
            SampleFormat::I16 => self.device.build_input_stream(
                self.config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    for frame in data.chunks_exact(channels) {
                        let _ = producer.push(frame[0] as f32 / i16::MAX as f32);
                    }
                },
                |err| eprintln!("audio stream error: {err}"),
                None,
            ),
            other => panic!("unsupported input sample format: {other:?}"),
        }
        .expect("failed to build input stream");
        stream.play().expect("failed to start input stream");
        SourceHandle::Live(stream)
    }
}

/// Replays a recorded clip through the same queue interface as `LiveCapture`, so every stage
/// downstream of the queue is integration-testable with no hardware and no terminal.
pub struct WavSource {
    sample_rate: u32,
    channels: u16,
    samples: Vec<f32>,
}

impl WavSource {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, hound::Error> {
        let mut reader = hound::WavReader::open(path)?;
        let spec = reader.spec();
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Int => {
                let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
                reader
                    .samples::<i32>()
                    .map(|s| s.map(|v| v as f32 / max))
                    .collect::<Result<_, _>>()?
            }
            hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        };
        Ok(Self {
            sample_rate: spec.sample_rate,
            channels: spec.channels,
            samples,
        })
    }
}

impl AudioSource for WavSource {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn start(self: Box<Self>, mut producer: Producer<f32>) -> SourceHandle {
        let channels = self.channels as usize;
        let samples = self.samples;
        let handle = std::thread::spawn(move || {
            for frame in samples.chunks_exact(channels) {
                let sample = frame[0];
                while producer.push(sample).is_err() {
                    std::thread::yield_now();
                }
            }
        });
        SourceHandle::Replay(handle)
    }
}
