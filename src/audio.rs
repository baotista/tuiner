//! The audio-source seam: live capture and a WAV reader that replays a recorded clip, behind one
//! interface. This is a testability decision, not a generality one — it lets every stage from
//! `detect` onward run against the committed `corpus/` deterministically, with no hardware.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use rtrb::Producer;

use crate::gate;
use crate::picker::DeviceEntry;

/// Feeds one Input Channel's samples into a lock-free queue.
pub trait AudioSource {
    fn sample_rate(&self) -> u32;

    /// Starts feeding samples into `producer`. The returned handle must be kept alive for as
    /// long as samples should keep flowing — dropping it stops the source.
    fn start(self: Box<Self>, producer: Producer<f32>) -> SourceHandle;
}

/// Keeps a started `AudioSource` alive.
pub enum SourceHandle {
    Live {
        stream: cpal::Stream,
        disconnected: Arc<AtomicBool>,
    },
    Replay(JoinHandle<()>),
}

impl SourceHandle {
    /// Whether the underlying stream reported an error — a device unplugged mid-session is the
    /// case this exists for. The picker's reopen path (`message: Some(..)`) is what the caller
    /// should show once this goes true.
    pub fn disconnected(&self) -> bool {
        match self {
            SourceHandle::Live { disconnected, .. } => disconnected.load(Ordering::Relaxed),
            SourceHandle::Replay(_) => false,
        }
    }
}

/// One Input Device as `cpal` exposes it, paired with the metadata the picker shows.
pub struct EnumeratedDevice {
    pub device: cpal::Device,
    pub info: DeviceEntry,
}

/// Every Input Device the default host currently exposes, each with its Input Channel count.
/// Devices that fail to report a name or a default input config (mid-enumeration disconnects,
/// output-only devices misreported by a host) are skipped rather than failing the whole list.
pub fn list_input_devices() -> Vec<EnumeratedDevice> {
    let host = cpal::default_host();
    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };
    devices
        .filter_map(|device| {
            let channels = device.default_input_config().ok()?.channels();
            let name = device.to_string();
            Some(EnumeratedDevice {
                device,
                info: DeviceEntry { name, channels },
            })
        })
        .collect()
}

/// A chosen Input Device, capturing in realtime and reading exactly one Input Channel.
pub struct LiveCapture {
    device: cpal::Device,
    config: StreamConfig,
    sample_format: SampleFormat,
    channel: usize,
}

impl LiveCapture {
    /// `channel` is the Input Channel to read; every other channel the device offers is decoded
    /// by the realtime callback and then discarded, never mixed in.
    pub fn open(device: cpal::Device, channel: usize) -> Result<Self, String> {
        let supported = device.default_input_config().map_err(|e| e.to_string())?;
        if channel >= supported.channels() as usize {
            return Err(format!(
                "Input Channel {channel} does not exist on this device ({} channels)",
                supported.channels()
            ));
        }
        Ok(Self {
            device,
            config: supported.config(),
            sample_format: supported.sample_format(),
            channel,
        })
    }
}

impl AudioSource for LiveCapture {
    fn sample_rate(&self) -> u32 {
        self.config.sample_rate
    }

    fn start(self: Box<Self>, mut producer: Producer<f32>) -> SourceHandle {
        let channels = self.config.channels as usize;
        let channel = self.channel;
        let disconnected = Arc::new(AtomicBool::new(false));
        let err_flag = disconnected.clone();
        let err_fn = move |err| {
            eprintln!("audio stream error: {err}");
            err_flag.store(true, Ordering::Relaxed);
        };
        // The realtime callback's one job: deinterleave the chosen Input Channel into the
        // queue. No allocation, no locking, no analysis — those happen on the consumer side.
        let stream = match self.sample_format {
            SampleFormat::F32 => self.device.build_input_stream(
                self.config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    for frame in data.chunks_exact(channels) {
                        let _ = producer.push(frame[channel]);
                    }
                },
                err_fn,
                None,
            ),
            SampleFormat::I16 => self.device.build_input_stream(
                self.config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    for frame in data.chunks_exact(channels) {
                        let _ = producer.push(frame[channel] as f32 / i16::MAX as f32);
                    }
                },
                err_fn,
                None,
            ),
            other => panic!("unsupported input sample format: {other:?}"),
        }
        .expect("failed to build input stream");
        stream.play().expect("failed to start input stream");
        SourceHandle::Live {
            stream,
            disconnected,
        }
    }
}

/// A live per-Input-Channel Level meter, streamed while the picker is open so the player can see
/// which jack their instrument is actually in. Independent of `Pipeline`: the picker has not
/// chosen a single Input Channel yet, so this reads every channel the Input Device offers.
pub struct LevelMeter {
    _stream: cpal::Stream,
    levels: Arc<[AtomicU32]>,
}

impl LevelMeter {
    pub fn start(device: &cpal::Device) -> Result<Self, String> {
        let supported = device.default_input_config().map_err(|e| e.to_string())?;
        let config = supported.config();
        let channels = config.channels as usize;
        let levels: Arc<[AtomicU32]> = (0..channels)
            .map(|_| AtomicU32::new(f32::NEG_INFINITY.to_bits()))
            .collect();

        let cb_levels = levels.clone();
        let stream = match supported.sample_format() {
            SampleFormat::F32 => device.build_input_stream(
                config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    update_levels(&cb_levels, data, channels, |s| s)
                },
                |err| eprintln!("level meter stream error: {err}"),
                None,
            ),
            SampleFormat::I16 => device.build_input_stream(
                config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    update_levels(&cb_levels, data, channels, |s| s as f32 / i16::MAX as f32)
                },
                |err| eprintln!("level meter stream error: {err}"),
                None,
            ),
            other => return Err(format!("unsupported input sample format: {other:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self {
            _stream: stream,
            levels,
        })
    }

    /// The current Level of every channel, in dBFS, most recent hop first-order — index matches
    /// the Input Channel index the picker shows.
    pub fn levels_db(&self) -> Vec<f32> {
        self.levels
            .iter()
            .map(|a| f32::from_bits(a.load(Ordering::Relaxed)))
            .collect()
    }
}

/// Deinterleaves one realtime buffer into per-channel Level, eased so the meter looks alive on a
/// pluck without flickering back to the floor between callbacks.
fn update_levels<T: Copy>(
    levels: &[AtomicU32],
    data: &[T],
    channels: usize,
    to_f32: impl Fn(T) -> f32,
) {
    let mut chan_buf: Vec<f32> = Vec::with_capacity(data.len() / channels.max(1));
    for c in 0..channels {
        chan_buf.clear();
        chan_buf.extend(data.chunks_exact(channels).map(|frame| to_f32(frame[c])));
        let target = gate::level_db(&chan_buf);
        let prev = f32::from_bits(levels[c].load(Ordering::Relaxed));
        let eased = gate::ease_level(prev, target);
        levels[c].store(eased.to_bits(), Ordering::Relaxed);
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
