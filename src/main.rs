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
use tuiner::{pitch, trail_capacity, tuning, window_samples};

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

/// Whether the app is naming bare Notes or matching against a chosen Tuning's Strings — toggled
/// with `Tab`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Chromatic,
    Guided,
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

    let tunings = tuning::all();
    let mut mode = Mode::Chromatic;
    let mut tuning_idx = 0usize;
    // Which String the player has named explicitly, so matching stops refusing to guess for a
    // fresh string that starts far too flat to fall inside any Capture Range. Only meaningful in
    // Guided Mode; a Tuning change clears it, since the Target Pitch it referred to has changed.
    let mut string_lock: Option<u8> = None;

    let mut readout = Readout::Listening { locked: None };
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
                KeyCode::Tab => {
                    mode = match mode {
                        Mode::Chromatic => Mode::Guided,
                        Mode::Guided => Mode::Chromatic,
                    }
                }
                KeyCode::Char('t') => {
                    tuning_idx = (tuning_idx + 1) % tunings.len();
                    string_lock = None;
                }
                KeyCode::Char(c @ '1'..='6') if mode == Mode::Guided => {
                    let n = c.to_digit(10).expect("guarded by '1'..='6'") as u8;
                    string_lock = toggle_string_lock(&tunings[tuning_idx], string_lock, n);
                }
                _ => {}
            }
        }

        match pipeline.poll() {
            Polled::Frame(frame) => {
                let live = matches!(frame, Frame::Pitched { .. });
                readout = match hz_reading(frame) {
                    Some((hz, dimmed)) => {
                        let named = name_pitch(mode, &tunings[tuning_idx], string_lock, hz);
                        if live {
                            trail.push(TrailSample::Deviation(named.cents));
                            let target = pitch::target_hz(hz, named.cents);
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
                            note: named.note,
                            hz,
                            cents: named.cents,
                            dimmed,
                            string_number: named.string_number,
                            locked: named.locked,
                            strobe_phase: strobe.phase(),
                            trail: trail.padded_samples(),
                        }
                    }
                    None => {
                        trail.push(TrailSample::Gap);
                        last_pitched = None;
                        Readout::Listening {
                            locked: locked_target(mode, &tunings[tuning_idx], string_lock),
                        }
                    }
                };
            }
            Polled::Ended => return TuningExit::DeviceLost,
            Polled::Pending => {}
        }

        let label = mode_label(mode, tunings[tuning_idx].name);
        terminal
            .draw(|f| ui::render(f, f.area(), &readout, &label))
            .expect("failed to draw frame");
    }
}

/// What `Tab`/`t` last landed on, for the border title — the only on-screen sign of the current
/// Mode and Tuning until the Cockpit's status area (issue #11) takes that job over.
fn mode_label(mode: Mode, tuning_name: &str) -> String {
    match mode {
        Mode::Chromatic => "Chromatic".to_string(),
        Mode::Guided => format!("Guided — {tuning_name}"),
    }
}

/// The hz/dimmed a `Frame` reads as, or `None` when there's nothing to show at all (`Unpitched`,
/// or `Silent` with no held reading yet).
fn hz_reading(frame: Frame) -> Option<(f32, bool)> {
    match frame {
        Frame::Pitched { hz, .. } => Some((hz, false)),
        Frame::Unpitched => None,
        Frame::Silent {
            held: Some((hz, _clarity)),
        } => Some((hz, true)),
        Frame::Silent { held: None } => None,
    }
}

/// What a sounding Pitch names, and how — a bare Note, or a String matched either by Capture
/// Range or by an active String Lock.
struct PitchName {
    note: String,
    cents: f32,
    string_number: Option<u8>,
    /// Whether `string_number` came from a String Lock overriding Capture Range matching, rather
    /// than a normal in-range match — the only thing distinguishing the two on screen.
    locked: bool,
}

/// The String named by an active String Lock, if any: only in Guided Mode, and only if `tuning`
/// still has a String numbered `string_lock` (it always will while a String Lock is set, since
/// changing Tuning clears it — but centralising the check here keeps that rule from being
/// restated wherever a caller needs to know whether a String Lock is currently in effect).
fn active_lock(
    mode: Mode,
    tuning: &tuning::Tuning,
    string_lock: Option<u8>,
) -> Option<&tuning::InstrumentString> {
    if mode != Mode::Guided {
        return None;
    }
    tuning.string(string_lock?)
}

/// Names `hz` per the current Mode: a bare Note in Chromatic Mode; in Guided Mode, the locked
/// String's Deviation however far `hz` is (ignoring `tuning`'s Capture Range) if a String Lock is
/// active, otherwise a String within Capture Range or the nearest Note as a fallback.
fn name_pitch(mode: Mode, tuning: &tuning::Tuning, string_lock: Option<u8>, hz: f32) -> PitchName {
    if let Some(s) = active_lock(mode, tuning, string_lock) {
        let Some(tuning::Match::String {
            number,
            note,
            cents,
        }) = tuning.match_locked(s.number, hz)
        else {
            unreachable!("active_lock only names a String this Tuning has")
        };
        return PitchName {
            note,
            cents,
            string_number: Some(number),
            locked: true,
        };
    }

    match mode {
        Mode::Chromatic => {
            let (note, cents) = pitch::nearest_note(hz, pitch::DEFAULT_REFERENCE_PITCH);
            PitchName {
                note,
                cents,
                string_number: None,
                locked: false,
            }
        }
        Mode::Guided => match tuning.match_pitch(hz) {
            tuning::Match::String {
                number,
                note,
                cents,
            } => PitchName {
                note,
                cents,
                string_number: Some(number),
                locked: false,
            },
            tuning::Match::Note { note, cents } => PitchName {
                note,
                cents,
                string_number: None,
                locked: false,
            },
        },
    }
}

/// The String Lock target for display purposes (String number and Note), shown even while
/// `Readout::Listening` so a fresh string that hasn't produced a stable Pitch yet still confirms
/// the Lock took effect. `None` outside Guided Mode, since a Chromatic readout has no String to
/// name.
fn locked_target(
    mode: Mode,
    tuning: &tuning::Tuning,
    string_lock: Option<u8>,
) -> Option<(u8, String)> {
    active_lock(mode, tuning, string_lock).map(|s| (s.number, s.note.clone()))
}

/// The String Lock state after digit key `n` is pressed: locks onto String `n` if `tuning` has
/// one, releases an existing Lock on that same String, or leaves the Lock untouched if `n` is
/// beyond `tuning`'s String count.
fn toggle_string_lock(tuning: &tuning::Tuning, current: Option<u8>, n: u8) -> Option<u8> {
    if tuning.string(n).is_none() {
        return current;
    }
    if current == Some(n) { None } else { Some(n) }
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
    fn chromatic_mode_label_names_no_tuning() {
        assert_eq!(mode_label(Mode::Chromatic, "DADGAD"), "Chromatic");
    }

    #[test]
    fn guided_mode_label_names_the_current_tuning() {
        assert_eq!(mode_label(Mode::Guided, "DADGAD"), "Guided — DADGAD");
    }

    fn bass_standard() -> tuning::Tuning {
        tuning::all()
            .into_iter()
            .find(|t| t.name == "Bass Standard")
            .unwrap()
    }

    #[test]
    fn a_number_key_locks_onto_the_corresponding_string() {
        let bass = bass_standard();
        assert_eq!(toggle_string_lock(&bass, None, 2), Some(2));
    }

    #[test]
    fn the_same_key_releases_an_existing_lock() {
        let bass = bass_standard();
        assert_eq!(toggle_string_lock(&bass, Some(2), 2), None);
    }

    #[test]
    fn a_different_key_switches_the_lock_to_the_new_string() {
        let bass = bass_standard();
        assert_eq!(toggle_string_lock(&bass, Some(2), 3), Some(3));
    }

    #[test]
    fn a_key_beyond_the_string_count_does_nothing() {
        // Bass Standard has four Strings — key 5 is out of range.
        let bass = bass_standard();
        assert_eq!(toggle_string_lock(&bass, None, 5), None);
        assert_eq!(toggle_string_lock(&bass, Some(2), 5), Some(2));
    }

    #[test]
    fn locked_target_is_none_in_chromatic_mode_even_with_a_lock_set() {
        let bass = bass_standard();
        assert_eq!(locked_target(Mode::Chromatic, &bass, Some(2)), None);
    }

    #[test]
    fn locked_target_names_the_string_and_note_in_guided_mode() {
        let bass = bass_standard();
        assert_eq!(
            locked_target(Mode::Guided, &bass, Some(2)),
            Some((2, "D2".to_string()))
        );
    }

    #[test]
    fn locked_target_is_none_when_nothing_is_locked() {
        let bass = bass_standard();
        assert_eq!(locked_target(Mode::Guided, &bass, None), None);
    }

    #[test]
    fn name_pitch_reports_the_locked_string_however_far_the_pitch_is() {
        let bass = bass_standard();
        // Far below D2 (String 2) — well outside any Capture Range, exactly the fresh-string
        // case a Lock exists for.
        let far_flat_of_d2 = pitch::midi_to_hz(38, pitch::DEFAULT_REFERENCE_PITCH) / 4.0;
        let named = name_pitch(Mode::Guided, &bass, Some(2), far_flat_of_d2);
        assert_eq!(named.note, "D2");
        assert_eq!(named.string_number, Some(2));
        assert!(named.locked);
        assert!(
            named.cents < -2000.0,
            "expected a very large flat Deviation, got {}",
            named.cents
        );
    }

    #[test]
    fn name_pitch_ignores_a_lock_in_chromatic_mode() {
        let bass = bass_standard();
        let a1_hz = pitch::midi_to_hz(33, pitch::DEFAULT_REFERENCE_PITCH);
        let named = name_pitch(Mode::Chromatic, &bass, Some(2), a1_hz);
        assert_eq!(named.note, "A1");
        assert_eq!(named.string_number, None);
        assert!(!named.locked);
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
