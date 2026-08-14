//! Wires the Input Device / Input Channel picker to the Chromatic Mode readout: capture the
//! chosen Input Channel, gate and smooth the sounding Pitch, and render it — nearest Note, Hz,
//! Deviation, and a coarse ±50 cent bar.
//!
//! Two threads and one lock-free queue, per the PRD's architecture: the realtime `cpal`
//! callback does one job — deinterleave into the queue — and this thread polls keyboard input,
//! drains samples, runs the pipeline, and renders, all in the same loop. Coupling analysis to
//! the render loop is affordable (well under 1% of a core) and keeps the failure mode benign: a
//! stall here just means a stale reading once it catches up.
//!
//! The outer loop below the picker and the tuning session share one path for "start tuning with
//! this Input Device and Input Channel" — reached whether the player just confirmed a fresh
//! pick, reopened the picker with `i`, or the Input Device vanished mid-session. That's
//! deliberate (PRD #5): a stale remembered device (future config work) will reopen the picker
//! through this same path.
//!
//! The Input Device list is re-enumerated every time the picker is about to be shown — never
//! cached across a reopen — so a vanished device actually disappears from the list and a freshly
//! plugged-in one actually appears. What survives a reopen is the concrete `cpal::Device` already
//! running, not an index into any particular enumeration, since indices shift as devices come and
//! go.

use std::env;
use std::time::{Duration, Instant};

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use rtrb::RingBuffer;
use tuiner::audio::{AudioSource, EnumeratedDevice, LevelMeter, LiveCapture, list_input_devices};
use tuiner::picker::{DeviceEntry, Outcome, Picker, Selection, Step};
use tuiner::pipeline::{Frame, Pipeline, Polled};
use tuiner::strobe::Strobe;
use tuiner::trail::{Trail, TrailSample};
use tuiner::ui::{self, PickerView, Readout};
use tuiner::{pitch, trail_capacity, window_samples};

fn main() {
    let startup_devices = list_input_devices();
    if startup_devices.is_empty() {
        println!("No Input Devices found — nothing to listen to.");
        return;
    }
    let startup_entries: Vec<DeviceEntry> =
        startup_devices.iter().map(|d| d.info.clone()).collect();

    let cli = match parse_cli_args(env::args().skip(1)) {
        Ok(cli) => cli,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(1);
        }
    };
    let cli_selection = match resolve_cli_selection(&cli, &startup_entries) {
        Ok(selection) => selection,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(1);
        }
    };
    // Resolved to a concrete Device right away, while `startup_devices` still matches what was
    // just validated against — nothing downstream needs to re-derive it from an index again.
    let mut pending: Option<(cpal::Device, usize)> =
        cli_selection.map(|s| (startup_devices[s.device_idx].device.clone(), s.channel_idx));

    let mut terminal = ratatui::init();
    let mut last_good = pending.clone();
    let mut cancellable = false;
    let mut message = None;
    let mut exit_message = None;

    loop {
        let (device, channel_idx) = match pending.take() {
            Some(pick) => pick,
            None => {
                let devices = list_input_devices();
                if devices.is_empty() {
                    exit_message =
                        Some("No Input Devices remain — nothing to listen to.".to_string());
                    break;
                }
                match run_picker(&mut terminal, &devices, cancellable, message.take()) {
                    PickerResult::Selected(selection) => (
                        devices[selection.device_idx].device.clone(),
                        selection.channel_idx,
                    ),
                    PickerResult::Cancelled => match last_good.clone() {
                        Some(pick) => pick,
                        None => break,
                    },
                    PickerResult::Quit => break,
                }
            }
        };
        last_good = Some((device.clone(), channel_idx));

        let source = match LiveCapture::open(device, channel_idx) {
            Ok(source) => source,
            Err(err) => {
                message = Some(err);
                cancellable = false;
                continue;
            }
        };

        match run_tuning(&mut terminal, source) {
            TuningExit::Quit => break,
            TuningExit::ReopenPicker => {
                cancellable = true;
                message = None;
            }
            TuningExit::DeviceLost => {
                // Nothing valid to cancel back to — the device that just vanished is the only
                // thing `last_good` holds, so Esc must not be allowed to re-select it.
                cancellable = false;
                message = Some("Input Device disappeared — pick again.".to_string());
            }
        }
    }

    ratatui::restore();
    if let Some(msg) = exit_message {
        println!("{msg}");
    }
}

/// Input Device / Input Channel selection taken from the command line, bypassing the picker.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct CliArgs {
    device: Option<usize>,
    channel: Option<usize>,
}

fn parse_cli_args(args: impl Iterator<Item = String>) -> Result<CliArgs, String> {
    let mut cli = CliArgs::default();
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--device" => {
                let value = args.next().ok_or("--device requires a value")?;
                cli.device = Some(
                    value
                        .parse()
                        .map_err(|_| format!("--device value '{value}' is not a number"))?,
                );
            }
            "--channel" => {
                let value = args.next().ok_or("--channel requires a value")?;
                cli.channel = Some(
                    value
                        .parse()
                        .map_err(|_| format!("--channel value '{value}' is not a number"))?,
                );
            }
            other => return Err(format!("unrecognized argument: {other}")),
        }
    }
    if cli.channel.is_some() && cli.device.is_none() {
        return Err("--channel requires --device to also be given".to_string());
    }
    Ok(cli)
}

/// Validates a parsed `CliArgs` against the Input Devices actually present. `Ok(None)` means no
/// `--device` was given at all, so the picker runs as normal. Takes the plain `DeviceEntry` list
/// rather than `EnumeratedDevice` so this validation is testable with fixtures, with no `cpal`
/// host required.
fn resolve_cli_selection(
    cli: &CliArgs,
    entries: &[DeviceEntry],
) -> Result<Option<Selection>, String> {
    let Some(device_idx) = cli.device else {
        return Ok(None);
    };
    if device_idx >= entries.len() {
        return Err(format!(
            "--device {device_idx} is out of range (0..{})",
            entries.len()
        ));
    }
    let channels = entries[device_idx].channels as usize;
    let channel_idx = cli.channel.unwrap_or(0);
    if channel_idx >= channels {
        return Err(format!(
            "--channel {channel_idx} is out of range for device {device_idx} ({channels} channel(s))"
        ));
    }
    Ok(Some(Selection {
        device_idx,
        channel_idx,
    }))
}

enum PickerResult {
    Selected(Selection),
    /// Backed out of a reopened picker with nothing chosen — the caller falls back to whatever
    /// was already running.
    Cancelled,
    Quit,
}

fn run_picker(
    terminal: &mut DefaultTerminal,
    devices: &[EnumeratedDevice],
    cancellable: bool,
    message: Option<String>,
) -> PickerResult {
    let entries: Vec<DeviceEntry> = devices.iter().map(|d| d.info.clone()).collect();
    let mut picker = Picker::new(entries, cancellable, message);
    let mut metered_idx = picker.device_idx();
    let mut meter = LevelMeter::start(&devices[metered_idx].device).ok();

    loop {
        if picker.step() == Step::Device && picker.device_idx() != metered_idx {
            metered_idx = picker.device_idx();
            meter = LevelMeter::start(&devices[metered_idx].device).ok();
        }

        let levels = meter.as_ref().map(|m| m.levels_db()).unwrap_or_default();
        let view = PickerView {
            devices: picker.devices(),
            step: picker.step(),
            device_idx: picker.device_idx(),
            channel_idx: picker.channel_idx(),
            levels_db: &levels,
            message: picker.message(),
        };
        terminal
            .draw(|f| ui::render_picker(f, f.area(), &view))
            .expect("failed to draw frame");

        if event::poll(Duration::from_millis(50)).unwrap_or(false)
            && let Ok(Event::Key(key)) = event::read()
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Up => picker.move_up(),
                KeyCode::Down => picker.move_down(),
                KeyCode::Char('q') => return PickerResult::Quit,
                KeyCode::Enter => {
                    if let Outcome::Selected(selection) = picker.confirm() {
                        return PickerResult::Selected(selection);
                    }
                }
                KeyCode::Esc => {
                    if let Outcome::Cancelled = picker.back_or_cancel() {
                        return PickerResult::Cancelled;
                    }
                }
                _ => {}
            }
        }
    }
}

enum TuningExit {
    Quit,
    /// The `i` key: the player wants to switch Input Device or Input Channel mid-session.
    ReopenPicker,
    /// The stream reported an error (a device unplugged) or the source ended outright.
    DeviceLost,
}

fn run_tuning(terminal: &mut DefaultTerminal, source: LiveCapture) -> TuningExit {
    let sample_rate = source.sample_rate();
    let window = window_samples(sample_rate);

    let (producer, consumer) = RingBuffer::<f32>::new(window * 4);
    let handle = Box::new(source).start(producer);
    let mut pipeline = Pipeline::new(consumer, sample_rate);

    let mut strobe = Strobe::new();
    // Set only while a Pitch is live, and to the moment it was last live — not wall-clock time
    // across a Silent gap. That keeps a genuine render stall (which leaves this untouched while
    // the string keeps sounding) distinguishable from the player simply not playing for a while
    // (which must not read as a stall once they start again).
    let mut last_pitched: Option<Instant> = None;
    let mut trail = Trail::new(trail_capacity());

    let mut readout = Readout::Listening;
    loop {
        if handle.disconnected() {
            return TuningExit::DeviceLost;
        }

        if event::poll(Duration::from_millis(10)).unwrap_or(false)
            && let Ok(Event::Key(key)) = event::read()
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return TuningExit::Quit,
                KeyCode::Char('i') => return TuningExit::ReopenPicker,
                _ => {}
            }
        }

        match pipeline.poll() {
            Polled::Frame(frame) => {
                let live = matches!(frame, Frame::Pitched { .. });
                readout = match note_reading(frame) {
                    Some((note, hz, cents, dimmed)) => {
                        if live {
                            trail.push(TrailSample::Deviation(cents));
                            let target = pitch::target_hz(hz, cents);
                            let now = Instant::now();
                            if let Some(prev) = last_pitched {
                                strobe.advance(hz, target, now.duration_since(prev).as_secs_f32());
                            }
                            last_pitched = Some(now);
                        } else {
                            // Silent-held: still a gap for the Trail — it reflects the hop's
                            // real classification, not what the readout happens to display.
                            trail.push(TrailSample::Gap);
                            last_pitched = None;
                        }
                        Readout::Reading {
                            note,
                            hz,
                            cents,
                            dimmed,
                            strobe_phase: strobe.phase(),
                            trail: trail.padded_samples(),
                        }
                    }
                    None => {
                        trail.push(TrailSample::Gap);
                        last_pitched = None;
                        Readout::Listening
                    }
                };
            }
            Polled::Ended => return TuningExit::DeviceLost,
            Polled::Pending => {}
        }

        terminal
            .draw(|f| ui::render(f, f.area(), &readout))
            .expect("failed to draw frame");
    }
}

/// The note/hz/cents/dimmed a `Frame` reads as, or `None` when there's nothing to show at all
/// (`Unpitched`, or `Silent` with no held reading yet).
fn note_reading(frame: Frame) -> Option<(String, f32, f32, bool)> {
    let (hz, dimmed) = match frame {
        Frame::Pitched { hz, .. } => (hz, false),
        Frame::Unpitched => return None,
        Frame::Silent {
            held: Some((hz, _clarity)),
        } => (hz, true),
        Frame::Silent { held: None } => return None,
    };
    let (note, cents) = pitch::nearest_note(hz, pitch::DEFAULT_REFERENCE_PITCH);
    Some((note, hz, cents, dimmed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> impl Iterator<Item = String> {
        v.iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn no_flags_parses_to_all_none() {
        assert_eq!(parse_cli_args(args(&[])).unwrap(), CliArgs::default());
    }

    #[test]
    fn device_and_channel_flags_parse() {
        let cli = parse_cli_args(args(&["--device", "1", "--channel", "2"])).unwrap();
        assert_eq!(cli.device, Some(1));
        assert_eq!(cli.channel, Some(2));
    }

    #[test]
    fn device_flag_alone_defaults_channel_to_none() {
        let cli = parse_cli_args(args(&["--device", "0"])).unwrap();
        assert_eq!(cli.device, Some(0));
        assert_eq!(cli.channel, None);
    }

    #[test]
    fn channel_without_device_is_an_error() {
        assert!(parse_cli_args(args(&["--channel", "0"])).is_err());
    }

    #[test]
    fn non_numeric_value_is_an_error() {
        assert!(parse_cli_args(args(&["--device", "banana"])).is_err());
    }

    #[test]
    fn missing_value_is_an_error() {
        assert!(parse_cli_args(args(&["--device"])).is_err());
    }

    #[test]
    fn unrecognized_flag_is_an_error() {
        assert!(parse_cli_args(args(&["--bogus"])).is_err());
    }

    fn entries(channels: &[u16]) -> Vec<DeviceEntry> {
        channels
            .iter()
            .enumerate()
            .map(|(i, &c)| DeviceEntry {
                name: format!("device-{i}"),
                channels: c,
            })
            .collect()
    }

    #[test]
    fn no_device_flag_resolves_to_none() {
        let entries = entries(&[1, 2]);
        let cli = CliArgs::default();
        assert_eq!(resolve_cli_selection(&cli, &entries), Ok(None));
    }

    #[test]
    fn device_and_channel_flags_resolve_to_a_selection() {
        let entries = entries(&[1, 2]);
        let cli = CliArgs {
            device: Some(1),
            channel: Some(1),
        };
        assert_eq!(
            resolve_cli_selection(&cli, &entries),
            Ok(Some(Selection {
                device_idx: 1,
                channel_idx: 1
            }))
        );
    }

    #[test]
    fn device_flag_without_channel_defaults_to_channel_zero() {
        let entries = entries(&[2]);
        let cli = CliArgs {
            device: Some(0),
            channel: None,
        };
        assert_eq!(
            resolve_cli_selection(&cli, &entries),
            Ok(Some(Selection {
                device_idx: 0,
                channel_idx: 0
            }))
        );
    }

    #[test]
    fn out_of_range_device_index_is_an_error() {
        let entries = entries(&[1]);
        let cli = CliArgs {
            device: Some(5),
            channel: None,
        };
        assert!(resolve_cli_selection(&cli, &entries).is_err());
    }

    #[test]
    fn out_of_range_channel_index_is_an_error() {
        let entries = entries(&[2]);
        let cli = CliArgs {
            device: Some(0),
            channel: Some(5),
        };
        assert!(resolve_cli_selection(&cli, &entries).is_err());
    }
}
