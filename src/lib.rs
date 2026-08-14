pub mod audio;
pub mod detect;
pub mod gate;
pub mod pipeline;
pub mod pitch;
pub mod smoothing;
pub mod ui;

/// Analysis window, sized in milliseconds rather than samples so 44.1 kHz and 48 kHz devices
/// need no special-casing — each just converts at its own reported rate.
pub const WINDOW_MS: f32 = 171.0;

/// How far apart consecutive analyses run, for a responsive readout without re-running on
/// every single sample.
pub const HOP_MS: f32 = 21.0;

/// Lowest Pitch the detector is asked to find — B0.
pub const MIN_HZ: f32 = 30.87;

/// Highest Pitch the detector is asked to find — E6.
pub const MAX_HZ: f32 = 1318.51;

/// Refinement low-pass multiple adopted per ADR 0001.
pub const LOWPASS_MULT: f32 = 8.0;

/// Converts [`WINDOW_MS`] to a sample count at the given device sample rate.
pub fn window_samples(sample_rate: u32) -> usize {
    (sample_rate as f32 * WINDOW_MS / 1000.0).round() as usize
}

/// Converts [`HOP_MS`] to a sample count at the given device sample rate.
pub fn hop_samples(sample_rate: u32) -> usize {
    (sample_rate as f32 * HOP_MS / 1000.0).round() as usize
}
