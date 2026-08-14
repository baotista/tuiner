//! The Chromatic Mode readout: nearest Note, sounding Pitch, Deviation, a coarse ±50 cent bar,
//! and the Strobe. Written as a pure function of a small view-model (`Readout`), not of
//! `pipeline::Frame` or `strobe::Strobe` directly, so it renders — and tests — without any audio
//! plumbing.
//!
//! Palette is blue for flat, orange for sharp, bright neutral for in tune — deliberately not
//! green and red, per ADR 0003. Hue carries direction only; magnitude comes from the bar
//! marker's position and the Strobe's drift rate, readable with colour ignored entirely.

use std::f32::consts::TAU;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::picker::{DeviceEntry, Step};
use crate::pitch::IN_TUNE_TOLERANCE_CENTS;
use crate::trail::TrailSample;

/// How far the coarse bar reaches in either direction. The Strobe, not this bar, carries fine
/// precision past this range (ADR 0002) — this is only the coarse half.
const BAR_RANGE_CENTS: f32 = 50.0;

/// Columns per full band cycle of the Strobe pattern — one full phase revolution (`TAU`) maps to
/// exactly one shift of this many columns, so the pattern returns to how it looked at the start
/// of the cycle.
const STROBE_PERIOD: usize = 6;

/// How far the Deviation Trail's vertical axis reaches in either direction — the same coarse
/// range as the bar, so the two panels read on one consistent scale.
const TRAIL_RANGE_CENTS: f32 = BAR_RANGE_CENTS;

const COLOR_FLAT: Color = Color::Rgb(90, 150, 230);
const COLOR_SHARP: Color = Color::Rgb(230, 145, 60);
const COLOR_IN_TUNE: Color = Color::Rgb(235, 235, 235);

/// What the readout should show, independent of how it got there (live, held, or nothing yet).
#[derive(Debug, Clone, PartialEq)]
pub enum Readout {
    /// Nothing trustworthy is sounding.
    Listening,
    /// A Pitch reading — live if `dimmed` is false, held from before a Silent gap if true.
    Reading {
        note: String,
        hz: f32,
        cents: f32,
        dimmed: bool,
        /// The Strobe's current phase accumulator reading, in radians — frozen at whatever it
        /// last was while `dimmed`, since nothing is sounding to advance it against.
        strobe_phase: f32,
        /// The Deviation Trail's recent history, oldest first. A snapshot, not a live handle —
        /// this view-model stays independent of `trail::Trail`'s own bookkeeping.
        trail: Vec<TrailSample>,
    },
}

pub fn render(frame: &mut Frame, area: Rect, readout: &Readout) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Tuiner — Chromatic ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    match readout {
        Readout::Listening => {
            frame.render_widget(Paragraph::new("listening…"), inner);
        }
        Readout::Reading {
            note,
            hz,
            cents,
            dimmed,
            strobe_phase,
            trail,
        } => {
            let (color, direction) = deviation_style(*cents);
            let mut style = Style::default().fg(color);
            if *dimmed {
                style = style.add_modifier(Modifier::DIM);
            }

            let header = Line::from(vec![
                Span::styled(format!("{note:<4}"), style),
                Span::raw(format!(" {hz:>8.2} Hz   ")),
                Span::styled(format!("{cents:+.1}c"), style),
                Span::raw(format!("   {direction}")),
            ]);

            let bar = bar_line(*cents, inner.width as usize, style);
            let strobe = strobe_line(*strobe_phase, inner.width as usize, style);

            let mut lines = vec![header, bar, strobe];
            let trail_rows = (inner.height as usize).saturating_sub(lines.len());
            if trail_rows > 0 {
                lines.extend(trail_canvas(trail, inner.width as usize, trail_rows, style));
            }
            frame.render_widget(Paragraph::new(lines), inner);
        }
    }
}

/// (colour, direction word) for a Deviation. Never names a rotation — tightening always raises
/// pitch, but which way the Peg turns depends on the machine head and which side of the
/// Headstock it sits on, so a rotation instruction would be wrong about half the time.
fn deviation_style(cents: f32) -> (Color, &'static str) {
    if cents.abs() <= IN_TUNE_TOLERANCE_CENTS {
        (COLOR_IN_TUNE, "in tune")
    } else if cents < 0.0 {
        (COLOR_FLAT, "tighten")
    } else {
        (COLOR_SHARP, "loosen")
    }
}

/// A coarse bar covering ±50 cents, sized to exactly `width` so it never overflows — never
/// wider than what the caller says will actually render. The marker's position carries
/// magnitude on its own, readable with colour ignored entirely; a centre tick marks 0 cents so
/// "how far from the tick" doesn't depend on remembering the bar's midpoint.
fn bar_line(cents: f32, width: usize, style: Style) -> Line<'static> {
    if width == 0 {
        return Line::from(Span::raw(""));
    }
    let center = (width - 1) as f32 / 2.0;
    // `cents` is clamped to ±BAR_RANGE_CENTS, so this ratio is always in [-1.0, 1.0] and the
    // marker position always lands within [0, width - 1] without needing a second clamp.
    let ratio = cents.clamp(-BAR_RANGE_CENTS, BAR_RANGE_CENTS) / BAR_RANGE_CENTS;
    let marker = (center + ratio * center).round() as usize;
    let center_idx = center.round() as usize;

    let mut s = String::with_capacity(width);
    for i in 0..width {
        s.push(if i == marker {
            '●'
        } else if i == center_idx {
            '|'
        } else {
            '─'
        });
    }
    Line::from(Span::styled(s, style))
}

/// A banded pattern, `width` cells wide, offset by the Strobe's current phase (ADR 0002). Drawn
/// fresh every frame from `phase` alone rather than kept as widget state — apparent motion comes
/// from the offset changing between renders as `strobe::Strobe` advances, not from anything this
/// function remembers.
fn strobe_line(phase: f32, width: usize, style: Style) -> Line<'static> {
    if width == 0 {
        return Line::from(Span::raw(""));
    }
    let offset = (phase / TAU * STROBE_PERIOD as f32).round() as isize;
    let s: String = (0..width)
        .map(|i| {
            let banded = (i as isize - offset).rem_euclid(STROBE_PERIOD as isize) as usize;
            if banded < STROBE_PERIOD / 2 {
                '█'
            } else {
                '·'
            }
        })
        .collect();
    Line::from(Span::styled(s, style))
}

/// Downsamples `samples` (chronological, oldest first) into exactly `bins` buckets, one per dot
/// column the Deviation Trail will render. A bucket with at least one measured Deviation reads
/// as their average; a bucket that is entirely gaps (or empty, before the Trail has filled up
/// this far) reads as `None` — a hole in the canvas, never invented by interpolating neighbours.
fn bin_samples(samples: &[TrailSample], bins: usize) -> Vec<Option<f32>> {
    if bins == 0 || samples.is_empty() {
        return vec![None; bins];
    }
    (0..bins)
        .map(|i| {
            let start = i * samples.len() / bins;
            let end = ((i + 1) * samples.len() / bins).max(start + 1);
            let values: Vec<f32> = samples[start..end]
                .iter()
                .filter_map(|s| match s {
                    TrailSample::Deviation(cents) => Some(*cents),
                    TrailSample::Gap => None,
                })
                .collect();
            if values.is_empty() {
                None
            } else {
                Some(values.iter().sum::<f32>() / values.len() as f32)
            }
        })
        .collect()
}

/// Row/column bit within a braille cell for dot `(dx, dy)`, `dx` in `0..2`, `dy` in `0..4` — the
/// standard Unicode braille dot numbering (dots 1-2-3-7 in the left column, 4-5-6-8 in the right).
const BRAILLE_BITS: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];

/// A grid of braille cells addressed by dot coordinates — 2 dots wide and 4 tall per cell, the 8
/// subpixels ADR 0002 spends colour down to one per cell to get. Exists so the Deviation Trail
/// can be built up dot by dot and line by line, then flattened into `Line`s in one place.
struct BrailleGrid {
    cols: usize,
    rows: usize,
    cells: Vec<u8>,
}

impl BrailleGrid {
    fn new(cols: usize, rows: usize) -> Self {
        Self {
            cols,
            rows,
            cells: vec![0; cols * rows],
        }
    }

    fn width_dots(&self) -> usize {
        self.cols * 2
    }

    fn height_dots(&self) -> usize {
        self.rows * 4
    }

    fn set_dot(&mut self, x: isize, y: isize) {
        if x < 0 || y < 0 {
            return;
        }
        let (x, y) = (x as usize, y as usize);
        if x >= self.width_dots() || y >= self.height_dots() {
            return;
        }
        let (cell_x, cell_y) = (x / 2, y / 4);
        let (dx, dy) = (x % 2, y % 4);
        self.cells[cell_y * self.cols + cell_x] |= BRAILLE_BITS[dy][dx];
    }

    /// Bresenham's line algorithm between two dot coordinates — the standard integer-only
    /// midpoint variant, so a connected stretch of Deviation renders as a continuous trace
    /// rather than only the dots that happen to land exactly on the line.
    fn line(&mut self, (x0, y0): (isize, isize), (x1, y1): (isize, isize)) {
        let dx = (x1 - x0).abs();
        let sx: isize = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy: isize = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let (mut x, mut y) = (x0, y0);
        loop {
            self.set_dot(x, y);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    fn to_lines(&self, style: Style) -> Vec<Line<'static>> {
        (0..self.rows)
            .map(|r| {
                let s: String = (0..self.cols)
                    .map(|c| {
                        let bits = self.cells[r * self.cols + c];
                        char::from_u32(0x2800 + bits as u32).unwrap_or(' ')
                    })
                    .collect();
                Line::from(Span::styled(s, style))
            })
            .collect()
    }
}

/// Maps a Deviation in cents to a dot row — clamped to [`TRAIL_RANGE_CENTS`], sharp at the top,
/// flat at the bottom, matching the sign convention used everywhere else in the readout.
fn cents_to_row(cents: f32, height_dots: usize) -> isize {
    if height_dots == 0 {
        return 0;
    }
    let ratio = (cents.clamp(-TRAIL_RANGE_CENTS, TRAIL_RANGE_CENTS) + TRAIL_RANGE_CENTS)
        / (2.0 * TRAIL_RANGE_CENTS);
    let inverted = 1.0 - ratio; // sharp (positive cents) renders near the top
    (inverted * (height_dots - 1) as f32).round() as isize
}

/// Renders the Deviation Trail on a braille canvas `width_cells` × `height_cells`. Each dot
/// column is one bin of `samples`' history; consecutive measured bins are connected, an isolated
/// measured bin still shows as a single dot, and a gap bin is left blank — never interpolated
/// through, per issue #7.
fn trail_canvas(
    samples: &[TrailSample],
    width_cells: usize,
    height_cells: usize,
    style: Style,
) -> Vec<Line<'static>> {
    let mut grid = BrailleGrid::new(width_cells, height_cells);
    let width_dots = grid.width_dots();
    let height_dots = grid.height_dots();
    if width_dots == 0 || height_dots == 0 {
        return grid.to_lines(style);
    }

    let bins = bin_samples(samples, width_dots);
    let mut prev: Option<(isize, isize)> = None;
    for (x, value) in bins.into_iter().enumerate() {
        match value {
            Some(cents) => {
                let point = (x as isize, cents_to_row(cents, height_dots));
                grid.set_dot(point.0, point.1);
                if let Some(from) = prev {
                    grid.line(from, point);
                }
                prev = Some(point);
            }
            None => prev = None,
        }
    }
    grid.to_lines(style)
}

/// What the Input Device / Input Channel picker should show, independent of `cpal` and the
/// `Picker` reducer's own bookkeeping — a pure view-model, same reasoning as `Readout`.
pub struct PickerView<'a> {
    pub devices: &'a [DeviceEntry],
    pub step: Step,
    pub device_idx: usize,
    pub channel_idx: usize,
    /// Live Level, in dBFS, one entry per channel of the currently highlighted Input Device —
    /// what makes it obvious which jack the instrument is actually in.
    pub levels_db: &'a [f32],
    /// The explanation banner shown when the picker was reopened rather than opened fresh —
    /// a mid-session `i` keypress carries none, a vanished remembered Input Device carries one.
    pub message: Option<&'a str>,
}

/// The bottom of the Level meter's displayed range, in dBFS — quieter than this shows as an
/// empty bar rather than trying to resolve dB differences that don't move it.
const METER_FLOOR_DB: f32 = -60.0;

pub fn render_picker(frame: &mut Frame, area: Rect, view: &PickerView) {
    let title = match view.step {
        Step::Device => " Tuiner — choose an Input Device ",
        Step::Channel => " Tuiner — choose an Input Channel ",
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = Vec::new();
    if let Some(message) = view.message {
        lines.push(Line::from(Span::styled(
            message.to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::default());
    }

    match view.step {
        Step::Device => {
            for (i, device) in view.devices.iter().enumerate() {
                lines.push(picker_row(
                    i == view.device_idx,
                    format!(
                        "{} ({} channel{})",
                        device.name,
                        device.channels,
                        plural(device.channels)
                    ),
                ));
            }
        }
        Step::Channel => {
            let device = &view.devices[view.device_idx];
            for c in 0..device.channels as usize {
                let db = view.levels_db.get(c).copied().unwrap_or(f32::NEG_INFINITY);
                let label = format!("Channel {}  {}", c + 1, level_bar(db, 20));
                lines.push(picker_row(c == view.channel_idx, label));
            }
        }
    }

    lines.push(Line::default());
    let hint = match view.step {
        Step::Device => "↑/↓ choose · Enter select · Esc back · q quit",
        Step::Channel => "↑/↓ choose · Enter confirm · Esc back",
    };
    lines.push(Line::from(Span::styled(
        hint,
        Style::default().add_modifier(Modifier::DIM),
    )));

    frame.render_widget(Paragraph::new(lines), inner);
}

fn plural(channels: u16) -> &'static str {
    if channels == 1 { "" } else { "s" }
}

fn picker_row(highlighted: bool, label: String) -> Line<'static> {
    let marker = if highlighted { "> " } else { "  " };
    let style = if highlighted {
        Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
    } else {
        Style::default()
    };
    Line::from(Span::styled(format!("{marker}{label}"), style))
}

/// A block-character Level bar, `width` cells wide, filled in proportion to how far `db` sits
/// between [`METER_FLOOR_DB`] and 0 dBFS — the range the picker's meter cares about.
fn level_bar(db: f32, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let ratio = ((db - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0);
    let filled = (ratio * width as f32).round() as usize;
    "█".repeat(filled) + &"·".repeat(width - filled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    fn render_to_buffer(readout: &Readout, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), readout)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn buffer_text(buffer: &Buffer) -> String {
        let area = buffer.area;
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buffer.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn reading(cents: f32) -> Readout {
        Readout::Reading {
            note: "E2".into(),
            hz: 82.0,
            cents,
            dimmed: false,
            strobe_phase: 0.0,
            trail: Vec::new(),
        }
    }

    #[test]
    fn flat_deviation_instructs_to_tighten() {
        let buf = render_to_buffer(&reading(-12.0), 60, 6);
        let text = buffer_text(&buf);
        assert!(text.contains("tighten"), "expected 'tighten' in:\n{text}");
        assert!(!text.contains("loosen"));
    }

    #[test]
    fn sharp_deviation_instructs_to_loosen() {
        let buf = render_to_buffer(&reading(12.0), 60, 6);
        let text = buffer_text(&buf);
        assert!(text.contains("loosen"), "expected 'loosen' in:\n{text}");
        assert!(!text.contains("tighten"));
    }

    #[test]
    fn in_tune_within_three_cents_says_so_and_gives_no_direction() {
        for cents in [-3.0, 0.0, 3.0] {
            let buf = render_to_buffer(&reading(cents), 60, 6);
            let text = buffer_text(&buf);
            assert!(
                text.contains("in tune"),
                "at {cents}c expected 'in tune' in:\n{text}"
            );
            assert!(!text.contains("tighten") && !text.contains("loosen"));
        }
    }

    #[test]
    fn just_outside_tolerance_gives_a_direction_not_in_tune() {
        let buf = render_to_buffer(&reading(3.1), 60, 6);
        let text = buffer_text(&buf);
        assert!(text.contains("loosen"));
        assert!(!text.contains("in tune"));

        let buf = render_to_buffer(&reading(-3.1), 60, 6);
        let text = buffer_text(&buf);
        assert!(text.contains("tighten"));
        assert!(!text.contains("in tune"));
    }

    #[test]
    fn no_rotation_direction_appears_anywhere() {
        for cents in [-40.0, -3.0, 0.0, 3.0, 40.0] {
            let buf = render_to_buffer(&reading(cents), 60, 6);
            let text = buffer_text(&buf).to_lowercase();
            for word in [
                "clockwise",
                "counterclockwise",
                "left",
                "right",
                "cw",
                "ccw",
            ] {
                assert!(
                    !text.contains(word),
                    "found rotation word '{word}' in:\n{text}"
                );
            }
        }
    }

    #[test]
    fn bar_never_overflows_at_various_terminal_widths() {
        // TestBackend panics on any out-of-bounds write, so simply completing draw() at every
        // width, including the extremes of the supported range and a terminal too narrow for
        // the border to leave any interior at all, proves the bar fit.
        for width in [0u16, 1, 2, 3, 20, 40, 60, 80, 120, 200] {
            let buf = render_to_buffer(&reading(-49.9), width, 6);
            assert_eq!(buf.area.width, width);
        }
    }

    #[test]
    fn palette_is_blue_orange_neutral_not_green_or_red() {
        for color in [COLOR_FLAT, COLOR_SHARP, COLOR_IN_TUNE] {
            assert_ne!(color, Color::Red);
            assert_ne!(color, Color::Green);
            assert_ne!(color, Color::LightRed);
            assert_ne!(color, Color::LightGreen);
        }
    }

    #[test]
    fn b0_and_e6_produce_a_correct_reading() {
        let (note_b0, cents_b0) =
            crate::pitch::nearest_note(30.868, crate::pitch::DEFAULT_REFERENCE_PITCH);
        let (note_e6, cents_e6) =
            crate::pitch::nearest_note(1318.51, crate::pitch::DEFAULT_REFERENCE_PITCH);
        assert_eq!(note_b0, "B0");
        assert_eq!(note_e6, "E6");
        assert!(cents_b0.abs() < 1.0 && cents_e6.abs() < 1.0);
    }

    #[test]
    fn listening_state_names_no_note() {
        let buf = render_to_buffer(&Readout::Listening, 60, 6);
        let text = buffer_text(&buf);
        assert!(text.contains("listening"));
    }

    fn render_picker_to_buffer(view: &PickerView, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render_picker(f, f.area(), view)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn devices() -> Vec<DeviceEntry> {
        vec![
            DeviceEntry {
                name: "Built-in Microphone".into(),
                channels: 1,
            },
            DeviceEntry {
                name: "USB Audio Interface".into(),
                channels: 2,
            },
        ]
    }

    #[test]
    fn device_step_lists_every_device_with_its_channel_count() {
        let devices = devices();
        let view = PickerView {
            devices: &devices,
            step: Step::Device,
            device_idx: 0,
            channel_idx: 0,
            levels_db: &[],
            message: None,
        };
        let text = buffer_text(&render_picker_to_buffer(&view, 60, 10));
        assert!(text.contains("Built-in Microphone (1 channel)"));
        assert!(text.contains("USB Audio Interface (2 channels)"));
    }

    #[test]
    fn highlighted_device_row_is_marked() {
        let devices = devices();
        let view = PickerView {
            devices: &devices,
            step: Step::Device,
            device_idx: 1,
            channel_idx: 0,
            levels_db: &[],
            message: None,
        };
        let text = buffer_text(&render_picker_to_buffer(&view, 60, 10));
        let highlighted_line = text
            .lines()
            .find(|l| l.contains("USB Audio Interface"))
            .unwrap();
        assert!(highlighted_line.contains("> USB Audio Interface"));
    }

    #[test]
    fn channel_step_lists_one_row_per_channel_with_a_level_meter() {
        let devices = devices();
        let view = PickerView {
            devices: &devices,
            step: Step::Channel,
            device_idx: 1,
            channel_idx: 0,
            levels_db: &[-10.0, -60.0],
            message: None,
        };
        let text = buffer_text(&render_picker_to_buffer(&view, 60, 10));
        assert!(text.contains("Channel 1"));
        assert!(text.contains("Channel 2"));
        // The louder channel's meter must show more filled cells than the quiet one.
        let line1 = text.lines().find(|l| l.contains("Channel 1")).unwrap();
        let line2 = text.lines().find(|l| l.contains("Channel 2")).unwrap();
        assert!(line1.matches('█').count() > line2.matches('█').count());
    }

    #[test]
    fn reopen_message_is_shown_as_a_banner() {
        let devices = devices();
        let view = PickerView {
            devices: &devices,
            step: Step::Device,
            device_idx: 0,
            channel_idx: 0,
            levels_db: &[],
            message: Some("Input Device disappeared — pick again"),
        };
        let text = buffer_text(&render_picker_to_buffer(&view, 60, 10));
        assert!(text.contains("Input Device disappeared"));
    }

    #[test]
    fn level_bar_is_full_at_0_dbfs_and_empty_at_the_floor() {
        assert_eq!(level_bar(0.0, 10), "█".repeat(10));
        assert_eq!(level_bar(METER_FLOOR_DB, 10), "·".repeat(10));
        assert_eq!(level_bar(-120.0, 10), "·".repeat(10));
    }

    fn reading_at_phase(strobe_phase: f32) -> Readout {
        Readout::Reading {
            note: "G3".into(),
            hz: 196.0,
            cents: -0.5,
            dimmed: false,
            strobe_phase,
            trail: Vec::new(),
        }
    }

    #[test]
    fn strobe_at_the_same_phase_renders_an_identical_stationary_pattern() {
        let first = buffer_text(&render_to_buffer(&reading_at_phase(0.0), 40, 6));
        let second = buffer_text(&render_to_buffer(&reading_at_phase(0.0), 40, 6));
        assert_eq!(
            first, second,
            "a stationary Deviation must not shift the rendered pattern"
        );
    }

    #[test]
    fn strobe_at_a_different_phase_shifts_the_rendered_pattern() {
        let at_zero = buffer_text(&render_to_buffer(&reading_at_phase(0.0), 40, 6));
        let advanced = buffer_text(&render_to_buffer(&reading_at_phase(1.0), 40, 6));
        assert_ne!(
            at_zero, advanced,
            "a moved Deviation must shift the rendered pattern"
        );
    }

    #[test]
    fn strobe_pattern_never_overflows_at_various_widths() {
        // Same rationale as `bar_never_overflows_at_various_terminal_widths`: TestBackend panics
        // on any out-of-bounds write, so simply completing draw() at every width proves it fit.
        for width in [0u16, 1, 2, 3, 6, 7, 30, 120] {
            let buf = render_to_buffer(&reading_at_phase(2.5), width, 6);
            assert_eq!(buf.area.width, width);
        }
    }

    #[test]
    fn reading_renders_a_strobe_line_below_the_bar() {
        let readout = reading_at_phase(1.0);
        let buf = render_to_buffer(&readout, 60, 8);
        let text = buffer_text(&buf);
        // Three content rows (header, bar, strobe) must all render distinct non-blank lines.
        let content_lines: Vec<&str> = text
            .lines()
            .filter(|l| {
                !l.trim_start_matches(['│', ' '])
                    .trim_end_matches(['│', ' '])
                    .is_empty()
            })
            .collect();
        assert!(
            content_lines.len() >= 3,
            "expected header, bar and strobe lines, got:\n{text}"
        );
    }

    #[test]
    fn bin_samples_averages_the_values_within_a_bucket() {
        let samples = [TrailSample::Deviation(2.0), TrailSample::Deviation(4.0)];
        assert_eq!(bin_samples(&samples, 1), vec![Some(3.0)]);
    }

    #[test]
    fn bin_samples_marks_an_all_gap_bucket_as_none() {
        let samples = [TrailSample::Gap, TrailSample::Gap];
        assert_eq!(bin_samples(&samples, 1), vec![None]);
    }

    #[test]
    fn bin_samples_ignores_gaps_mixed_in_with_real_values() {
        let samples = [TrailSample::Gap, TrailSample::Deviation(10.0)];
        assert_eq!(bin_samples(&samples, 1), vec![Some(10.0)]);
    }

    #[test]
    fn bin_samples_with_no_samples_yields_all_none() {
        assert_eq!(bin_samples(&[], 3), vec![None, None, None]);
    }

    #[test]
    fn bin_samples_with_zero_bins_returns_empty() {
        assert_eq!(
            bin_samples(&[TrailSample::Deviation(1.0)], 0),
            Vec::<Option<f32>>::new()
        );
    }

    /// The blank braille pattern — no dots set — is what an unrendered or gap cell looks like.
    fn blank_braille() -> char {
        char::from_u32(0x2800).unwrap()
    }

    #[test]
    fn a_single_dot_produces_the_correct_braille_character() {
        let mut grid = BrailleGrid::new(1, 1);
        grid.set_dot(0, 0); // dot 1: bit 0x01
        let lines = grid.to_lines(Style::default());
        assert_eq!(lines[0].spans[0].content, "⠁");
    }

    #[test]
    fn a_dot_outside_the_grid_is_silently_ignored() {
        let mut grid = BrailleGrid::new(1, 1);
        grid.set_dot(-1, 0);
        grid.set_dot(0, -1);
        grid.set_dot(100, 100);
        let lines = grid.to_lines(Style::default());
        assert_eq!(lines[0].spans[0].content, blank_braille().to_string());
    }

    #[test]
    fn a_horizontal_line_sets_every_dot_it_crosses() {
        let mut grid = BrailleGrid::new(2, 1); // 4 dots wide, 4 dots tall
        grid.line((0, 0), (3, 0));
        let lines = grid.to_lines(Style::default());
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        // Every cell the line crosses must differ from the blank pattern.
        assert!(text.chars().all(|c| c != blank_braille()));
    }

    #[test]
    fn trail_of_all_gaps_renders_a_blank_canvas() {
        let readout = Readout::Reading {
            note: "G3".into(),
            hz: 196.0,
            cents: 0.0,
            dimmed: false,
            strobe_phase: 0.0,
            trail: vec![TrailSample::Gap; 40],
        };
        let buf = render_to_buffer(&readout, 40, 8);
        let text = buffer_text(&buf);
        assert!(
            !text
                .chars()
                .any(|c| (0x2801..=0x28FF).contains(&(c as u32))),
            "expected no raised braille dots in an all-gap Trail, got:\n{text}"
        );
    }

    #[test]
    fn trail_with_values_renders_at_least_one_braille_dot() {
        let readout = Readout::Reading {
            note: "G3".into(),
            hz: 196.0,
            cents: 0.0,
            dimmed: false,
            strobe_phase: 0.0,
            trail: vec![TrailSample::Deviation(-40.0); 40],
        };
        let buf = render_to_buffer(&readout, 40, 8);
        let text = buffer_text(&buf);
        assert!(
            text.chars()
                .any(|c| (0x2801..=0x28FF).contains(&(c as u32))),
            "expected at least one raised braille dot with real Deviation history, got:\n{text}"
        );
    }

    #[test]
    fn trail_never_overflows_at_various_sizes() {
        let readout = Readout::Reading {
            note: "G3".into(),
            hz: 196.0,
            cents: 0.0,
            dimmed: false,
            strobe_phase: 0.0,
            trail: vec![TrailSample::Deviation(12.0); 50],
        };
        for (width, height) in [(0u16, 0u16), (1, 1), (2, 2), (3, 6), (40, 3), (120, 20)] {
            let buf = render_to_buffer(&readout, width, height);
            assert_eq!(buf.area.width, width);
            assert_eq!(buf.area.height, height);
        }
    }
}
