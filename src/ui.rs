//! The tuning readout: nearest Note or matched String, sounding Pitch, Deviation, a coarse ±50
//! cent bar, the Strobe, the Deviation Trail, and — in Guided Mode — the Headstock sprite and
//! String panel. Written as a pure function of a small view-model (`Readout`), not of
//! `pipeline::Frame`, `strobe::Strobe`, or `tuning::Tuning` directly, so it renders — and tests —
//! without any audio plumbing.
//!
//! Palette is blue for flat, orange for sharp, bright neutral for in tune — deliberately not
//! green and red, per ADR 0003. Hue carries direction only; magnitude comes from the bar
//! marker's position and the Strobe's drift rate, readable with colour ignored entirely. The
//! Headstock's Pegs carry the same rule further: each `StringStatus` is a distinct filled shape,
//! not just a distinct colour, per ADR 0002's half-block/braille split (half-blocks here, since
//! each Peg needs its own colour — braille spends colour down to one per cell).

use std::f32::consts::TAU;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::picker::{DeviceEntry, Step};
use crate::pitch::IN_TUNE_TOLERANCE_CENTS;
use crate::trail::TrailSample;
use crate::tuning::StringStatus;

/// Below this width or height, refuse outright (issue #11) rather than render a Cockpit too
/// cramped to be legible.
const MIN_SUPPORTED_WIDTH: u16 = 40;
const MIN_SUPPORTED_HEIGHT: u16 = 12;

/// Width of the Headstock sidebar column, wide enough for the sprite, a join gap, and the panel.
const HEADSTOCK_SIDEBAR_WIDTH: u16 = 24;

/// Chromatic Mode's "simpler centred layout" (PRD): a fixed-width column centred in whatever
/// room is available, rather than the full-width main column Guided Mode's sidebar leaves behind.
const CHROMATIC_MAIN_WIDTH: u16 = 60;

/// Degradation thresholds, in inner (post-border) rows/columns. Each is nested inside the one
/// before it — Trail's condition requires Headstock-space's, which requires the coarse bar's —
/// so the drop order issue #11 specifies (Trail, then Headstock, then the bar) holds by
/// construction, regardless of the exact numbers chosen here. Note, Deviation and the Strobe
/// never appear in this ladder at all: they are the irreducible core and are always drawn.
///
/// `BAR_MIN_HEIGHT` is deliberately set just above `MIN_SUPPORTED_HEIGHT`'s inner floor (10) —
/// at the smallest supported size the bar must already be gone too, leaving exactly the
/// irreducible core the PRD names, not the core-plus-bar a lower threshold would leave behind.
const BAR_MIN_HEIGHT: usize = 11; // header + Strobe + bar, with a little headroom besides
const HEADSTOCK_MIN_WIDTH: usize = 60; // the sidebar plus a still-legible main column
const HEADSTOCK_MIN_HEIGHT: usize = 12;
const TRAIL_MIN_HEIGHT: usize = 16; // enough headroom that the Trail is worth showing at all

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

/// One String's row on the Headstock panel and its Peg on the sprite.
#[derive(Debug, Clone, PartialEq)]
pub struct StringView {
    pub number: u8,
    pub note: String,
    pub status: StringStatus,
}

/// The Headstock sprite and String panel's data, present only in Guided Mode — Chromatic Mode
/// has no Tuning, and so no Strings to show. Carried on both `Readout` variants so progress stays
/// visible whether or not a Pitch happens to be sounding this instant.
#[derive(Debug, Clone, PartialEq)]
pub struct HeadstockView {
    pub strings: Vec<StringView>,
}

/// What the readout should show, independent of how it got there (live, held, or nothing yet).
#[derive(Debug, Clone, PartialEq)]
pub enum Readout {
    /// Nothing trustworthy is sounding. `locked` names an active String Lock (String number and
    /// Note) even here — a fresh string is exactly the case a Lock exists for, and it often
    /// won't produce a stable Pitch the moment the player engages it.
    Listening {
        locked: Option<(u8, String)>,
        headstock: Option<HeadstockView>,
    },
    /// A Pitch reading — live if `dimmed` is false, held from before a Silent gap if true.
    Reading {
        note: String,
        hz: f32,
        cents: f32,
        dimmed: bool,
        /// `Some(n)` in Guided Mode when the Pitch matched String `n` — by Capture Range, or by
        /// an active String Lock when `locked` is true. `None` in Chromatic Mode, or in Guided
        /// Mode when nothing matched and `note` names the nearest Note instead.
        string_number: Option<u8>,
        /// Whether `string_number` came from a String Lock overriding Capture Range matching,
        /// rather than a normal in-range match — the only thing distinguishing the two on
        /// screen, since both show the same String and Note.
        locked: bool,
        /// The Strobe's current phase accumulator reading, in radians — frozen at whatever it
        /// last was while `dimmed`, since nothing is sounding to advance it against.
        strobe_phase: f32,
        /// The Deviation Trail's recent history, oldest first. A snapshot, not a live handle —
        /// this view-model stays independent of `trail::Trail`'s own bookkeeping.
        trail: Vec<TrailSample>,
        headstock: Option<HeadstockView>,
    },
}

/// Whether the coarse bar, the Headstock sidebar, and the Deviation Trail each have room, given
/// inner (post-border) dimensions. Nested — `trail` requires `headstock_space`, which requires
/// `bar` — so the drop order issue #11 specifies (Trail first, then Headstock, then the bar)
/// holds by construction: it is never possible for a lower-priority panel's field to be true
/// while a higher-priority one's is false. Note, Deviation and the Strobe never appear here at
/// all — they are the irreducible core and are always drawn regardless of size.
struct DegradationTiers {
    bar: bool,
    headstock_space: bool,
    trail: bool,
}

fn degradation_tiers(inner_width: usize, inner_height: usize) -> DegradationTiers {
    let bar = inner_height >= BAR_MIN_HEIGHT;
    let headstock_space =
        bar && inner_width >= HEADSTOCK_MIN_WIDTH && inner_height >= HEADSTOCK_MIN_HEIGHT;
    let trail = headstock_space && inner_height >= TRAIL_MIN_HEIGHT;
    DegradationTiers {
        bar,
        headstock_space,
        trail,
    }
}

/// Chromatic Mode's "simpler centred layout" (PRD): a fixed-width column centred within
/// whatever room is available, rather than the full-width main column Guided Mode's sidebar
/// leaves behind.
fn centered_area(area: Rect, width: u16) -> Rect {
    let width = width.min(area.width);
    let margin = (area.width - width) / 2;
    Rect {
        x: area.x + margin,
        y: area.y,
        width,
        height: area.height,
    }
}

/// `mode_label` names the border title, e.g. `"Chromatic"` or `"Guided — Guitar Standard"` —
/// the only on-screen sign of which Mode and Tuning `Tab`/`t` last landed on.
pub fn render(frame: &mut Frame, area: Rect, readout: &Readout, mode_label: &str) {
    if area.width < MIN_SUPPORTED_WIDTH || area.height < MIN_SUPPORTED_HEIGHT {
        render_too_small(frame, area);
        return;
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Tuiner — {mode_label} "));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let tiers = degradation_tiers(inner.width as usize, inner.height as usize);

    match readout {
        Readout::Listening { locked, headstock } => {
            render_listening(frame, inner, &tiers, locked, headstock)
        }
        Readout::Reading {
            note,
            hz,
            cents,
            dimmed,
            string_number,
            locked,
            strobe_phase,
            trail,
            headstock,
        } => render_reading(
            frame,
            inner,
            &tiers,
            &ReadingView {
                note,
                hz: *hz,
                cents: *cents,
                dimmed: *dimmed,
                string_number: *string_number,
                locked: *locked,
                strobe_phase: *strobe_phase,
                trail,
                headstock,
            },
        ),
    }
}

/// A plain message naming the size Tuiner needs, rather than rendering a Cockpit too cramped to
/// be legible (issue #11). No border: `area` may be smaller than a border could even occupy.
fn render_too_small(frame: &mut Frame, area: Rect) {
    let message = format!(
        "Terminal too small for Tuiner — need at least {MIN_SUPPORTED_WIDTH}x{MIN_SUPPORTED_HEIGHT}, have {}x{}.",
        area.width, area.height
    );
    // Wrapped, not clipped: a narrow-enough terminal would otherwise cut the message off before
    // it ever names the size actually needed — the one thing this screen exists to say.
    frame.render_widget(Paragraph::new(message).wrap(Wrap { trim: false }), area);
}

/// One line of the keymap recap overlay: the key(s) that trigger a binding and what it does.
pub struct KeyBinding {
    pub keys: &'static str,
    pub description: &'static str,
}

/// Every binding currently wired up — the one place the recap overlay reads from, so a future
/// binding only needs adding here to appear in the overlay too.
pub const KEYMAP: &[KeyBinding] = &[
    KeyBinding {
        keys: "Tab",
        description: "toggle Mode",
    },
    KeyBinding {
        keys: "t",
        description: "cycle Tuning",
    },
    KeyBinding {
        keys: "1-6",
        description: "String Lock",
    },
    KeyBinding {
        keys: "+ / -",
        description: "adjust Reference Pitch",
    },
    KeyBinding {
        keys: "i",
        description: "reopen input picker",
    },
    KeyBinding {
        keys: "?",
        description: "toggle this keymap",
    },
    KeyBinding {
        keys: "q / Esc",
        description: "quit",
    },
];

/// The keymap recap overlay (issue #13): a bordered box covering the whole frame, listing every
/// `KEYMAP` binding. Drawn as an extra widget after `render`'s own draw call, on top of whatever
/// readout is already on screen — the caller keeps polling and drawing the readout underneath at
/// its usual rate, so dismissing the overlay shows a live reading rather than a stale one.
///
/// Below `MIN_SUPPORTED_WIDTH`/`MIN_SUPPORTED_HEIGHT` this draws nothing at all, the same floor
/// `render` enforces — otherwise it would cover up `render_too_small`'s message with a box no more
/// legible than what it's hiding.
pub fn render_keymap_overlay(frame: &mut Frame, area: Rect) {
    if area.width < MIN_SUPPORTED_WIDTH || area.height < MIN_SUPPORTED_HEIGHT {
        return;
    }
    frame.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).title(" Keymap ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line> = KEYMAP
        .iter()
        .map(|b| Line::from(format!("{:<8}{}", b.keys, b.description)))
        .collect();
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// Renders the Headstock sidebar (if `tiers.headstock_space` allows it and Guided Mode has one)
/// and returns the remaining main column — centred in Chromatic Mode's simpler layout, full
/// width otherwise — for the caller to fill in. Named for the side effect as well as the
/// geometry: unlike this file's other `*_line`/`*_lines` helpers, this one draws.
fn render_headstock_sidebar_and_split(
    frame: &mut Frame,
    inner: Rect,
    tiers: &DegradationTiers,
    headstock: &Option<HeadstockView>,
) -> Rect {
    match headstock {
        Some(view) if tiers.headstock_space => {
            let cols = Layout::horizontal([
                Constraint::Length(HEADSTOCK_SIDEBAR_WIDTH),
                Constraint::Min(0),
            ])
            .split(inner);
            frame.render_widget(Paragraph::new(headstock_and_panel_lines(view)), cols[0]);
            cols[1]
        }
        Some(_) => inner, // Guided Mode, but no room for the sidebar this size.
        None => centered_area(inner, CHROMATIC_MAIN_WIDTH), // Chromatic Mode.
    }
}

/// A small, unobtrusive reminder that `?` opens the keymap recap overlay (issue #13) — otherwise
/// nothing on screen ever names that key, and a first-time player has no way to discover it
/// (issue #15). Part of both readout states' irreducible core: always drawn, never subject to the
/// degradation ladder, the same way Note/Deviation/the Strobe already are.
fn keymap_hint_line() -> Line<'static> {
    Line::from(Span::styled(
        "? for keymap",
        Style::default().add_modifier(Modifier::DIM),
    ))
}

fn render_listening(
    frame: &mut Frame,
    inner: Rect,
    tiers: &DegradationTiers,
    locked: &Option<(u8, String)>,
    headstock: &Option<HeadstockView>,
) {
    let main = render_headstock_sidebar_and_split(frame, inner, tiers, headstock);

    let mut lines = vec![Line::from("listening…")];
    if let Some((number, note)) = locked {
        lines.push(Line::from(format!(
            "[Locked: {}]",
            string_label(*number, note)
        )));
    }
    lines.push(keymap_hint_line());
    frame.render_widget(Paragraph::new(lines), main);
}

/// Everything `render_reading` needs from a `Readout::Reading` — one struct instead of nine
/// positional parameters, since every field here is already grouped exactly this way on the
/// enum variant itself.
struct ReadingView<'a> {
    note: &'a str,
    hz: f32,
    cents: f32,
    dimmed: bool,
    string_number: Option<u8>,
    locked: bool,
    strobe_phase: f32,
    trail: &'a [TrailSample],
    headstock: &'a Option<HeadstockView>,
}

fn render_reading(frame: &mut Frame, inner: Rect, tiers: &DegradationTiers, view: &ReadingView) {
    let main = render_headstock_sidebar_and_split(frame, inner, tiers, view.headstock);

    let (color, direction) = deviation_style(view.cents);
    let mut style = Style::default().fg(color);
    if view.dimmed {
        style = style.add_modifier(Modifier::DIM);
    }

    let label = match view.string_number {
        Some(n) if view.locked => format!("[L] {}", string_label(n, view.note)),
        Some(n) => string_label(n, view.note),
        None => view.note.to_string(),
    };
    let header = Line::from(vec![
        Span::styled(format!("{label:<10}"), style),
        Span::raw(format!(" {:>8.2} Hz   ", view.hz)),
        Span::styled(format!("{:+.1}c", view.cents), style),
        Span::raw(format!("   {direction}")),
    ]);

    let main_width = main.width as usize;
    let mut lines = vec![header];
    if tiers.bar {
        lines.push(bar_line(view.cents, main_width, style));
    }
    lines.push(strobe_line(view.strobe_phase, main_width, style));
    lines.push(keymap_hint_line());

    if tiers.trail {
        let remaining = (main.height as usize).saturating_sub(lines.len());
        if remaining > 0 {
            lines.extend(trail_canvas(view.trail, main_width, remaining, style));
        }
    }

    frame.render_widget(Paragraph::new(lines), main);
}

/// The String and Note together, e.g. `"Str 5 A2"` — used both for a matched Reading and for
/// naming an active String Lock while `Listening`, so the two states agree on one format.
fn string_label(number: u8, note: &str) -> String {
    format!("Str {number} {note}")
}

const PEG_UNTOUCHED: Color = Color::Rgb(90, 90, 90);
const PEG_SOUNDING: Color = Color::Rgb(230, 200, 80);
const HEADSTOCK_WOOD: Color = Color::Rgb(120, 90, 60);

/// Each Peg is a `PEG_SIZE` × `PEG_SIZE` nub, reused for all three `StringStatus` shapes.
const PEG_SIZE: usize = 3;
/// Vertical pixel distance between one Peg's top and the next — a six-in-line (or four-in-line)
/// Headstock's defining feature, all Pegs mounted along one straight edge in String order.
const PEG_SPACING: usize = 4;
/// Blank pixel columns between the body's edge and the Pegs, so they read as sticking out of it
/// rather than fused to its side.
const PEG_GAP: usize = 1;
const BODY_WIDTH: usize = 7;
/// Body rows above the first Peg and below the last, so the row of Pegs doesn't run flush with
/// either end of the body.
const BODY_MARGIN: usize = 2;
/// Rows over which the body linearly tapers down to the neck's width — the paddle silhouette a
/// real Headstock has, rather than a plain rectangle.
const TAPER_HEIGHT: usize = 4;
const NECK_WIDTH: usize = 3;
const NECK_HEIGHT: usize = 4;

/// A pixel grid rendered two rows at a time via half-block characters (▀/▄), which keep two
/// independently coloured subpixels per cell — the trick ADR 0002 reserves for the Headstock
/// sprite, since braille spends colour down to one per cell instead. An unset pixel falls through
/// to the terminal's own background rather than forcing a colour.
struct PixelGrid {
    width: usize,
    height: usize,
    pixels: Vec<Option<Color>>,
}

impl PixelGrid {
    fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![None; width * height],
        }
    }

    fn set(&mut self, x: usize, y: usize, color: Color) {
        if x < self.width && y < self.height {
            self.pixels[y * self.width + x] = Some(color);
        }
    }

    fn get(&self, x: usize, y: usize) -> Option<Color> {
        if x < self.width && y < self.height {
            self.pixels[y * self.width + x]
        } else {
            None
        }
    }

    fn to_lines(&self) -> Vec<Line<'static>> {
        (0..self.height)
            .step_by(2)
            .map(|y| {
                let spans: Vec<Span<'static>> = (0..self.width)
                    .map(|x| {
                        let top = self.get(x, y);
                        let bottom = self.get(x, y + 1);
                        match (top, bottom) {
                            (Some(t), Some(b)) => Span::styled("▀", Style::default().fg(t).bg(b)),
                            (Some(t), None) => Span::styled("▀", Style::default().fg(t)),
                            (None, Some(b)) => Span::styled("▄", Style::default().fg(b)),
                            (None, None) => Span::raw(" "),
                        }
                    })
                    .collect();
                Line::from(spans)
            })
            .collect()
    }
}

/// The Headstock sprite: a vertical paddle tapering into a neck stub, with one Peg per String
/// mounted along its right edge in a straight line, top to bottom — the actual defining shape of
/// a six-in-line or four-in-line Headstock (all tuners on one edge, in String order), rather than
/// pegs sitting on top of a block. `strings.len()` alone decides how many Pegs there are, so one
/// parametric shape covers both instruments instead of two hardcoded ones. Each Peg's fill shape,
/// not just its colour, differs by `StringStatus` so state reads with colour ignored entirely: a
/// small dot (`Untouched`), a solid block (`Sounding`), or a cross (`InTune`).
fn headstock_sprite_lines(strings: &[StringView]) -> Vec<Line<'static>> {
    let peg_count = strings.len();
    let peg_run_height = peg_count.saturating_sub(1) * PEG_SPACING + PEG_SIZE;
    let body_height = BODY_MARGIN * 2 + peg_run_height;
    let height = body_height + TAPER_HEIGHT + NECK_HEIGHT;
    let width = BODY_WIDTH + PEG_GAP + PEG_SIZE;
    let mut grid = PixelGrid::new(width, height);

    // The body: full `BODY_WIDTH` through the straight run the Pegs mount along, then tapering
    // to the neck's width — the paddle silhouette a real Headstock has.
    for y in 0..body_height {
        for x in 0..BODY_WIDTH {
            grid.set(x, y, HEADSTOCK_WOOD);
        }
    }
    for row in 0..TAPER_HEIGHT {
        let t = row as f32 / (TAPER_HEIGHT.saturating_sub(1).max(1)) as f32;
        let row_width = (BODY_WIDTH as f32 - (BODY_WIDTH - NECK_WIDTH) as f32 * t).round() as usize;
        for x in 0..row_width {
            grid.set(x, body_height + row, HEADSTOCK_WOOD);
        }
    }
    for y in (body_height + TAPER_HEIGHT)..height {
        for x in 0..NECK_WIDTH {
            grid.set(x, y, HEADSTOCK_WOOD);
        }
    }

    // Pegs, sticking out from the body's right edge — one per String, evenly spaced top to
    // bottom in String order, `PEG_GAP` blank columns clear of the body so they read as
    // protruding from it rather than fused to its side.
    let peg_x = BODY_WIDTH + PEG_GAP;
    for (i, s) in strings.iter().enumerate() {
        let top = BODY_MARGIN + i * PEG_SPACING;
        match s.status {
            StringStatus::Untouched => grid.set(peg_x + 1, top + 1, PEG_UNTOUCHED),
            StringStatus::Sounding => {
                for dy in 0..PEG_SIZE {
                    for dx in 0..PEG_SIZE {
                        grid.set(peg_x + dx, top + dy, PEG_SOUNDING);
                    }
                }
            }
            StringStatus::InTune => {
                // A cross, not a solid block: half-block rendering only changes *character*
                // (not just colour) where a whole cell's top-and-bottom pixel pattern differs
                // from its neighbours, so leaving the corners unset — not just one hidden pixel
                // in the middle of an otherwise-solid block — is what makes this distinguishable
                // from `Sounding` with colour ignored entirely, not only by a colour difference.
                for dy in 0..PEG_SIZE {
                    for dx in 0..PEG_SIZE {
                        if dx == 1 || dy == 1 {
                            grid.set(peg_x + dx, top + dy, COLOR_IN_TUNE);
                        }
                    }
                }
            }
        }
    }

    grid.to_lines()
}

/// String number, Note, and status symbol — a distinct glyph per `StringStatus` (`·`/`●`/`✓`)
/// on top of the colour, so the panel reads with colour ignored too.
fn string_panel_lines(strings: &[StringView]) -> Vec<Line<'static>> {
    strings
        .iter()
        .map(|s| {
            let (color, symbol) = match s.status {
                StringStatus::Untouched => (PEG_UNTOUCHED, '·'),
                StringStatus::Sounding => (PEG_SOUNDING, '●'),
                StringStatus::InTune => (COLOR_IN_TUNE, '✓'),
            };
            Line::from(Span::styled(
                format!("{:>2} {:<4}{symbol}", s.number, s.note),
                Style::default().fg(color),
            ))
        })
        .collect()
}

/// The sprite and panel side by side, row for row — compact enough to share the same lines
/// budget as the Trail beneath the coarse readout.
fn headstock_and_panel_lines(view: &HeadstockView) -> Vec<Line<'static>> {
    let sprite = headstock_sprite_lines(&view.strings);
    let panel = string_panel_lines(&view.strings);
    let rows = sprite.len().max(panel.len());
    (0..rows)
        .map(|i| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            if let Some(l) = sprite.get(i) {
                spans.extend(l.spans.iter().cloned());
            }
            spans.push(Span::raw("   "));
            if let Some(l) = panel.get(i) {
                spans.extend(l.spans.iter().cloned());
            }
            Line::from(spans)
        })
        .collect()
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

    fn render_keymap_overlay_to_buffer(width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_keymap_overlay(f, f.area()))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn keymap_overlay_lists_every_binding() {
        let text = buffer_text(&render_keymap_overlay_to_buffer(60, 20));
        for binding in KEYMAP {
            assert!(
                text.contains(binding.keys) && text.contains(binding.description),
                "expected '{}: {}' in:\n{text}",
                binding.keys,
                binding.description
            );
        }
    }

    #[test]
    fn keymap_overlay_fits_the_smallest_supported_terminal_size_without_overflow() {
        // TestBackend panics on any out-of-bounds write, so simply completing draw() at exactly
        // the smallest supported size proves the overlay doesn't overflow it.
        let buf = render_keymap_overlay_to_buffer(MIN_SUPPORTED_WIDTH, MIN_SUPPORTED_HEIGHT);
        let text = buffer_text(&buf);
        for binding in KEYMAP {
            assert!(
                text.contains(binding.keys),
                "expected '{}' to still fit at the minimum supported size, got:\n{text}",
                binding.keys
            );
        }
    }

    #[test]
    fn keymap_overlay_never_overflows_at_various_supported_terminal_sizes() {
        for (width, height) in [(40u16, 12u16), (60, 14), (80, 24), (120, 36)] {
            let buf = render_keymap_overlay_to_buffer(width, height);
            assert_eq!(buf.area.width, width);
            assert_eq!(buf.area.height, height);
        }
    }

    #[test]
    fn keymap_overlay_draws_nothing_below_the_minimum_supported_size() {
        // Below MIN_SUPPORTED_WIDTH/HEIGHT, `render` shows the "terminal too small" message
        // instead of the readout — the overlay must not cover it up with a box of its own.
        let buf =
            render_keymap_overlay_to_buffer(MIN_SUPPORTED_WIDTH - 1, MIN_SUPPORTED_HEIGHT - 1);
        let text = buffer_text(&buf);
        assert!(
            !text.contains("Keymap"),
            "expected no overlay content below the minimum supported size, got:\n{text}"
        );
    }

    fn render_to_buffer(readout: &Readout, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render(f, f.area(), readout, "Chromatic"))
            .unwrap();
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
            string_number: None,
            locked: false,
            strobe_phase: 0.0,
            trail: Vec::new(),
            headstock: None,
        }
    }

    #[test]
    fn flat_deviation_instructs_to_tighten() {
        let buf = render_to_buffer(&reading(-12.0), 60, 14);
        let text = buffer_text(&buf);
        assert!(text.contains("tighten"), "expected 'tighten' in:\n{text}");
        assert!(!text.contains("loosen"));
    }

    #[test]
    fn sharp_deviation_instructs_to_loosen() {
        let buf = render_to_buffer(&reading(12.0), 60, 14);
        let text = buffer_text(&buf);
        assert!(text.contains("loosen"), "expected 'loosen' in:\n{text}");
        assert!(!text.contains("tighten"));
    }

    #[test]
    fn in_tune_within_three_cents_says_so_and_gives_no_direction() {
        for cents in [-3.0, 0.0, 3.0] {
            let buf = render_to_buffer(&reading(cents), 60, 14);
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
        let buf = render_to_buffer(&reading(3.1), 60, 14);
        let text = buffer_text(&buf);
        assert!(text.contains("loosen"));
        assert!(!text.contains("in tune"));

        let buf = render_to_buffer(&reading(-3.1), 60, 14);
        let text = buffer_text(&buf);
        assert!(text.contains("tighten"));
        assert!(!text.contains("in tune"));
    }

    #[test]
    fn no_rotation_direction_appears_anywhere() {
        for cents in [-40.0, -3.0, 0.0, 3.0, 40.0] {
            let buf = render_to_buffer(&reading(cents), 60, 14);
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
    fn bar_line_never_overflows_at_extreme_widths() {
        // TestBackend panics on any out-of-bounds write, so simply completing draw() at every
        // width, including down to zero, proves `bar_line` itself never writes past its width —
        // exercised directly since `render()` now refuses anything below `MIN_SUPPORTED_WIDTH`,
        // making these extremes unreachable through the public entry point.
        for width in [0usize, 1, 2, 3, 20, 39] {
            let backend = TestBackend::new(width as u16, 1);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|f| {
                    let line = bar_line(-49.9, width, Style::default());
                    f.render_widget(Paragraph::new(line), f.area());
                })
                .unwrap();
        }
    }

    #[test]
    fn bar_never_overflows_at_various_supported_terminal_widths() {
        for width in [40u16, 60, 80, 120, 200] {
            let buf = render_to_buffer(&reading(-49.9), width, 14);
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
        let buf = render_to_buffer(
            &Readout::Listening {
                locked: None,
                headstock: None,
            },
            60,
            14,
        );
        let text = buffer_text(&buf);
        assert!(text.contains("listening"));
    }

    #[test]
    fn listening_state_shows_an_active_lock() {
        let buf = render_to_buffer(
            &Readout::Listening {
                locked: Some((5, "A2".into())),
                headstock: None,
            },
            60,
            14,
        );
        let text = buffer_text(&buf);
        assert!(text.contains("Locked") && text.contains("Str 5 A2"));
    }

    // --- Issue #15: keymap recap hint ---

    #[test]
    fn reading_shows_a_hint_naming_the_keymap_key() {
        let buf = render_to_buffer(&reading(0.0), 60, 14);
        let text = buffer_text(&buf);
        assert!(
            text.contains('?') && text.contains("keymap"),
            "expected a hint naming the keymap key in:\n{text}"
        );
    }

    #[test]
    fn listening_shows_a_hint_naming_the_keymap_key() {
        let buf = render_to_buffer(
            &Readout::Listening {
                locked: None,
                headstock: None,
            },
            60,
            14,
        );
        let text = buffer_text(&buf);
        assert!(
            text.contains('?') && text.contains("keymap"),
            "expected a hint naming the keymap key in:\n{text}"
        );
    }

    #[test]
    fn reading_shows_the_hint_in_guided_mode_too() {
        let readout = Readout::Reading {
            note: "A2".into(),
            hz: 110.0,
            cents: 0.0,
            dimmed: false,
            string_number: Some(5),
            locked: false,
            strobe_phase: 0.0,
            trail: Vec::new(),
            headstock: Some(sample_headstock()),
        };
        let buf = render_to_buffer(&readout, 70, 14);
        let text = buffer_text(&buf);
        assert!(
            text.contains('?') && text.contains("keymap"),
            "expected the hint in Guided Mode too, got:\n{text}"
        );
    }

    #[test]
    fn listening_shows_the_hint_in_guided_mode_too() {
        let buf = render_to_buffer(
            &Readout::Listening {
                locked: None,
                headstock: Some(sample_headstock()),
            },
            70,
            14,
        );
        let text = buffer_text(&buf);
        assert!(
            text.contains('?') && text.contains("keymap"),
            "expected the hint in Guided Mode too, got:\n{text}"
        );
    }

    #[test]
    fn keymap_hint_still_shows_at_the_smallest_supported_size() {
        let buf = render_to_buffer(&reading(0.0), MIN_SUPPORTED_WIDTH, MIN_SUPPORTED_HEIGHT);
        let text = buffer_text(&buf);
        assert!(
            text.contains("keymap"),
            "expected the keymap hint even at the floor size, got:\n{text}"
        );
    }

    #[test]
    fn keymap_hint_survives_when_the_trail_fills_its_exact_height_budget() {
        // At the precise threshold where the Trail tier turns on (inner 60x16), the Trail's
        // `remaining` row budget must already have subtracted the hint's row — otherwise the
        // Trail claims that row for itself and the hint gets clipped out of view rather than the
        // Trail simply rendering one row shorter.
        let readout = guided_reading(Some(sample_headstock()), sample_trail());
        let buf = render_to_buffer(&readout, 62, 18); // inner: 60x16, exactly TRAIL_MIN_HEIGHT
        let text = buffer_text(&buf);
        assert!(
            text.contains("keymap"),
            "expected the hint to survive the Trail's height budget, got:\n{text}"
        );
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
            string_number: None,
            locked: false,
            strobe_phase,
            trail: Vec::new(),
            headstock: None,
        }
    }

    #[test]
    fn strobe_at_the_same_phase_renders_an_identical_stationary_pattern() {
        let first = buffer_text(&render_to_buffer(&reading_at_phase(0.0), 40, 14));
        let second = buffer_text(&render_to_buffer(&reading_at_phase(0.0), 40, 14));
        assert_eq!(
            first, second,
            "a stationary Deviation must not shift the rendered pattern"
        );
    }

    #[test]
    fn strobe_at_a_different_phase_shifts_the_rendered_pattern() {
        let at_zero = buffer_text(&render_to_buffer(&reading_at_phase(0.0), 40, 14));
        let advanced = buffer_text(&render_to_buffer(&reading_at_phase(1.0), 40, 14));
        assert_ne!(
            at_zero, advanced,
            "a moved Deviation must shift the rendered pattern"
        );
    }

    #[test]
    fn strobe_line_never_overflows_at_extreme_widths() {
        // Exercised directly, same rationale as `bar_line_never_overflows_at_extreme_widths`:
        // `render()` now refuses anything below `MIN_SUPPORTED_WIDTH`, so widths this small are
        // otherwise unreachable through the public entry point.
        for width in [0usize, 1, 2, 3, 6, 7, 30, 39] {
            let backend = TestBackend::new(width as u16, 1);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|f| {
                    let line = strobe_line(2.5, width, Style::default());
                    f.render_widget(Paragraph::new(line), f.area());
                })
                .unwrap();
        }
    }

    #[test]
    fn strobe_pattern_never_overflows_at_various_supported_widths() {
        for width in [40u16, 60, 80, 120] {
            let buf = render_to_buffer(&reading_at_phase(2.5), width, 14);
            assert_eq!(buf.area.width, width);
        }
    }

    #[test]
    fn reading_renders_a_strobe_line_below_the_bar() {
        let readout = reading_at_phase(1.0);
        let buf = render_to_buffer(&readout, 60, 14);
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
            string_number: None,
            locked: false,
            strobe_phase: 0.0,
            trail: vec![TrailSample::Gap; 40],
            headstock: None,
        };
        let buf = render_to_buffer(&readout, 70, 20);
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
            string_number: None,
            locked: false,
            strobe_phase: 0.0,
            trail: vec![TrailSample::Deviation(-40.0); 40],
            headstock: None,
        };
        let buf = render_to_buffer(&readout, 70, 20);
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
            string_number: None,
            locked: false,
            strobe_phase: 0.0,
            trail: vec![TrailSample::Deviation(12.0); 50],
            headstock: None,
        };
        for (width, height) in [
            (0u16, 0u16),
            (1, 1),
            (2, 2),
            (3, 6),
            (40, 3),
            (40, 12),
            (70, 20),
            (120, 36),
        ] {
            let buf = render_to_buffer(&readout, width, height);
            assert_eq!(buf.area.width, width);
            assert_eq!(buf.area.height, height);
        }
    }

    #[test]
    fn guided_mode_names_the_string_and_its_note_together() {
        let readout = Readout::Reading {
            note: "A2".into(),
            hz: 110.0,
            cents: 0.0,
            dimmed: false,
            string_number: Some(5),
            locked: false,
            strobe_phase: 0.0,
            trail: Vec::new(),
            headstock: None,
        };
        let text = buffer_text(&render_to_buffer(&readout, 60, 14));
        assert!(
            text.contains("Str 5") && text.contains("A2"),
            "expected both the String number and its Note in:\n{text}"
        );
    }

    #[test]
    fn chromatic_mode_names_only_the_note_with_no_string() {
        let text = buffer_text(&render_to_buffer(&reading(0.0), 60, 14));
        assert!(
            !text.contains("Str "),
            "expected no String label in:\n{text}"
        );
    }

    #[test]
    fn the_border_title_names_the_current_mode() {
        let backend = TestBackend::new(60, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render(f, f.area(), &reading(0.0), "Guided — DADGAD"))
            .unwrap();
        let text = buffer_text(&terminal.backend().buffer().clone());
        assert!(
            text.contains("Guided — DADGAD"),
            "expected the Mode/Tuning label in the title, got:\n{text}"
        );
    }

    fn one_string(status: StringStatus) -> Vec<StringView> {
        vec![StringView {
            number: 1,
            note: "E2".into(),
            status,
        }]
    }

    #[test]
    fn headstock_sprite_has_the_right_number_of_pegs_for_guitar_and_bass() {
        let guitar: Vec<StringView> = (1..=6)
            .map(|n| StringView {
                number: n,
                note: "E2".into(),
                status: StringStatus::Untouched,
            })
            .collect();
        let bass: Vec<StringView> = (1..=4)
            .map(|n| StringView {
                number: n,
                note: "E1".into(),
                status: StringStatus::Untouched,
            })
            .collect();

        // Both instruments share one Peg column at the body's right edge — six-in-line and
        // four-in-line differ in how many Pegs run down it, i.e. the sprite's height, not width.
        let expected_width = BODY_WIDTH + PEG_GAP + PEG_SIZE;
        let guitar_lines = headstock_sprite_lines(&guitar);
        let bass_lines = headstock_sprite_lines(&bass);
        for line in &guitar_lines {
            assert_eq!(line.width(), expected_width);
        }
        for line in &bass_lines {
            assert_eq!(line.width(), expected_width);
        }
        assert!(
            guitar_lines.len() > bass_lines.len(),
            "six Pegs must run longer down the edge than four"
        );
    }

    #[test]
    fn headstock_peg_shapes_differ_by_status_with_colour_ignored() {
        let render = |status| {
            headstock_sprite_lines(&one_string(status))
                .iter()
                .map(|l| {
                    l.spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let untouched = render(StringStatus::Untouched);
        let sounding = render(StringStatus::Sounding);
        let in_tune = render(StringStatus::InTune);
        assert_ne!(
            untouched, sounding,
            "Untouched and Sounding must render differently"
        );
        assert_ne!(
            untouched, in_tune,
            "Untouched and InTune must render differently"
        );
        assert_ne!(
            sounding, in_tune,
            "Sounding and InTune must render differently"
        );
    }

    #[test]
    fn string_panel_uses_a_distinct_symbol_per_status() {
        let untouched = string_panel_lines(&one_string(StringStatus::Untouched))[0].clone();
        let sounding = string_panel_lines(&one_string(StringStatus::Sounding))[0].clone();
        let in_tune = string_panel_lines(&one_string(StringStatus::InTune))[0].clone();
        let text_of = |l: &Line| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        };
        assert!(text_of(&untouched).contains('·'));
        assert!(text_of(&sounding).contains('●'));
        assert!(text_of(&in_tune).contains('✓'));
    }

    #[test]
    fn string_panel_names_the_string_number_and_note() {
        let line = &string_panel_lines(&one_string(StringStatus::Untouched))[0];
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains('1'));
        assert!(text.contains("E2"));
    }

    fn headstock_view(strings: Vec<StringView>) -> HeadstockView {
        HeadstockView { strings }
    }

    #[test]
    fn reading_with_a_headstock_renders_the_sprite_and_panel() {
        let readout = Readout::Reading {
            note: "A2".into(),
            hz: 110.0,
            cents: 0.0,
            dimmed: false,
            string_number: Some(1),
            locked: false,
            strobe_phase: 0.0,
            trail: Vec::new(),
            headstock: Some(headstock_view(vec![StringView {
                number: 1,
                note: "A2".into(),
                status: StringStatus::InTune,
            }])),
        };
        let text = buffer_text(&render_to_buffer(&readout, 70, 14));
        assert!(
            text.contains('✓'),
            "expected the panel's status symbol in:\n{text}"
        );
    }

    #[test]
    fn listening_with_a_headstock_still_renders_it() {
        let readout = Readout::Listening {
            locked: None,
            headstock: Some(headstock_view(vec![StringView {
                number: 3,
                note: "G3".into(),
                status: StringStatus::Sounding,
            }])),
        };
        let text = buffer_text(&render_to_buffer(&readout, 70, 14));
        assert!(
            text.contains('●'),
            "expected the panel's status symbol in:\n{text}"
        );
    }

    #[test]
    fn headstock_never_overflows_at_various_sizes() {
        let readout = Readout::Reading {
            note: "A2".into(),
            hz: 110.0,
            cents: 0.0,
            dimmed: false,
            string_number: Some(1),
            locked: false,
            strobe_phase: 0.0,
            trail: Vec::new(),
            headstock: Some(headstock_view(
                (1..=6)
                    .map(|n| StringView {
                        number: n,
                        note: "E2".into(),
                        status: StringStatus::Untouched,
                    })
                    .collect(),
            )),
        };
        for (width, height) in [(0u16, 0u16), (1, 1), (10, 3), (40, 8), (120, 20)] {
            let buf = render_to_buffer(&readout, width, height);
            assert_eq!(buf.area.width, width);
            assert_eq!(buf.area.height, height);
        }
    }

    // --- Issue #11: Cockpit assembly and small-terminal degradation ---

    fn sample_headstock() -> HeadstockView {
        headstock_view(vec![
            StringView {
                number: 1,
                note: "E4".into(),
                status: StringStatus::InTune,
            },
            StringView {
                number: 2,
                note: "B3".into(),
                status: StringStatus::Sounding,
            },
        ])
    }

    fn sample_trail() -> Vec<TrailSample> {
        vec![TrailSample::Deviation(10.0); 60]
    }

    fn guided_reading(headstock: Option<HeadstockView>, trail: Vec<TrailSample>) -> Readout {
        Readout::Reading {
            note: "G3".into(),
            hz: 196.0,
            cents: 10.0,
            dimmed: false,
            string_number: Some(3),
            locked: false,
            strobe_phase: 1.0,
            trail,
            headstock,
        }
    }

    fn has_braille_dot(text: &str) -> bool {
        text.chars()
            .any(|c| (0x2801..=0x28FF).contains(&(c as u32)))
    }

    #[test]
    fn cockpit_at_120x36_renders_every_panel() {
        let readout = guided_reading(Some(sample_headstock()), sample_trail());
        let text = buffer_text(&render_to_buffer(&readout, 120, 36));
        assert!(text.contains("Str 3"), "expected the header, got:\n{text}");
        assert!(
            text.contains('─') || text.contains('|'),
            "expected the coarse bar, got:\n{text}"
        );
        assert!(text.contains('█'), "expected the Strobe, got:\n{text}");
        assert!(
            text.contains('▀') || text.contains('▄'),
            "expected the Headstock sprite, got:\n{text}"
        );
        assert!(
            text.contains("E4") && text.contains('✓'),
            "expected the String panel, got:\n{text}"
        );
        assert!(
            has_braille_dot(&text),
            "expected the Deviation Trail, got:\n{text}"
        );
    }

    #[test]
    fn chromatic_mode_centres_content_with_a_visible_margin() {
        let readout = reading(5.0); // the `reading` helper's headstock is always None
        let text = buffer_text(&render_to_buffer(&readout, 120, 20));
        let header_line = text.lines().find(|l| l.contains("Hz")).unwrap();
        let after_border: String = header_line
            .chars()
            .skip_while(|c| *c != '│')
            .skip(1)
            .collect();
        let leading_spaces = after_border.chars().take_while(|c| *c == ' ').count();
        assert!(
            leading_spaces > 5,
            "expected a centred left margin, got {leading_spaces} spaces in:\n{header_line}"
        );
        assert!(
            !text.contains("Str "),
            "Chromatic Mode must show no Headstock/String label"
        );
    }

    #[test]
    fn centered_area_centers_within_available_width() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 10,
        };
        let centered = centered_area(area, 60);
        assert_eq!(centered.width, 60);
        assert_eq!(centered.x, 20);
    }

    #[test]
    fn centered_area_never_exceeds_the_available_width() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 30,
            height: 10,
        };
        let centered = centered_area(area, 60);
        assert_eq!(centered.width, 30);
        assert_eq!(centered.x, 0);
    }

    #[test]
    fn degradation_tiers_are_strictly_nested() {
        // Trail's condition must never hold unless Headstock-space's does too, which must never
        // hold unless the bar's does — regardless of the specific thresholds, by construction.
        for w in (0..=140).step_by(4) {
            for h in (0..=40).step_by(2) {
                let tiers = degradation_tiers(w, h);
                assert!(
                    !tiers.trail || tiers.headstock_space,
                    "trail without headstock-space at {w}x{h}"
                );
                assert!(
                    !tiers.headstock_space || tiers.bar,
                    "headstock-space without bar at {w}x{h}"
                );
            }
        }
    }

    #[test]
    fn full_size_shows_bar_headstock_and_trail() {
        let tiers = degradation_tiers(118, 34); // 120x36 inner
        assert!(tiers.bar && tiers.headstock_space && tiers.trail);
    }

    #[test]
    fn medium_terminal_drops_the_trail_but_keeps_the_headstock_and_bar() {
        let readout = guided_reading(Some(sample_headstock()), sample_trail());
        let text = buffer_text(&render_to_buffer(&readout, 70, 16));
        assert!(
            !has_braille_dot(&text),
            "Trail should be dropped first, got:\n{text}"
        );
        assert!(
            text.contains("E4") && text.contains('✓'),
            "Headstock/panel should still show, got:\n{text}"
        );
        assert!(
            text.contains('─') || text.contains('|'),
            "the coarse bar should still show, got:\n{text}"
        );
    }

    #[test]
    fn narrow_terminal_drops_the_headstock_and_trail_but_keeps_the_bar() {
        let readout = guided_reading(Some(sample_headstock()), sample_trail());
        let text = buffer_text(&render_to_buffer(&readout, 50, 30));
        assert!(
            !text.contains("E4"),
            "Headstock/panel should be dropped next, got:\n{text}"
        );
        assert!(
            !has_braille_dot(&text),
            "Trail must already be gone too (nested), got:\n{text}"
        );
        assert!(
            text.contains('─') || text.contains('|'),
            "the coarse bar should still show, got:\n{text}"
        );
    }

    #[test]
    fn note_deviation_and_strobe_survive_at_the_smallest_supported_size() {
        let readout = guided_reading(None, Vec::new());
        let text = buffer_text(&render_to_buffer(&readout, 40, 12));
        assert!(
            text.contains("G3") && text.contains("196.00 Hz") && text.contains("+10.0c"),
            "expected the Note and Deviation at the floor, got:\n{text}"
        );
        assert!(
            text.contains('█'),
            "expected the Strobe at the floor, got:\n{text}"
        );
    }

    #[test]
    fn the_coarse_bar_is_also_dropped_at_the_smallest_supported_size() {
        // Completes the drop order at the floor: Trail and Headstock are already gone by 40x12
        // (both need more room than that), and the bar goes too, leaving exactly the irreducible
        // core the PRD names — not the core plus a bar a looser threshold would leave behind.
        // ASCII '|' (the bar's centre tick) is checked rather than '─', since the outer border
        // already draws '─' regardless of whether the bar itself is shown.
        let readout = guided_reading(None, Vec::new());
        let text = buffer_text(&render_to_buffer(&readout, 40, 12));
        assert!(
            !text.contains('|'),
            "expected the coarse bar to be dropped at the floor too, got:\n{text}"
        );
    }

    #[test]
    fn at_exactly_the_minimum_size_the_app_still_renders_normally() {
        let text = buffer_text(&render_to_buffer(&reading(0.0), 40, 12));
        assert!(
            text.contains("Hz"),
            "expected normal content at the exact floor, got:\n{text}"
        );
        assert!(!text.contains("need at least"));
    }

    #[test]
    fn below_the_minimum_width_the_app_states_the_size_it_needs() {
        let text = buffer_text(&render_to_buffer(&reading(0.0), 39, 20));
        assert!(
            text.contains("40") && text.contains("12"),
            "expected the required size in the message, got:\n{text}"
        );
        assert!(
            !text.contains("Hz"),
            "expected no normal readout content, got:\n{text}"
        );
    }

    #[test]
    fn below_the_minimum_height_the_app_states_the_size_it_needs() {
        let text = buffer_text(&render_to_buffer(&reading(0.0), 60, 11));
        assert!(text.contains("40") && text.contains("12"));
        assert!(!text.contains("Hz"));
    }

    #[test]
    fn a_tiny_terminal_does_not_panic() {
        for (w, h) in [(0u16, 0u16), (1, 1), (5, 3), (39, 11)] {
            let buf = render_to_buffer(&reading(0.0), w, h);
            assert_eq!(buf.area.width, w);
            assert_eq!(buf.area.height, h);
        }
    }

    #[test]
    fn resizing_mid_session_relayouts_without_corruption() {
        let readout = guided_reading(Some(sample_headstock()), sample_trail());
        let backend = TestBackend::new(120, 36);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render(f, f.area(), &readout, "Guided — Test"))
            .unwrap();

        for (w, h) in [
            (40u16, 12u16),
            (10, 5),
            (70, 16),
            (120, 36),
            (39, 11),
            (80, 24),
        ] {
            terminal.backend_mut().resize(w, h);
            terminal
                .draw(|f| render(f, f.area(), &readout, "Guided — Test"))
                .unwrap();
            let buf = terminal.backend().buffer();
            assert_eq!(buf.area.width, w);
            assert_eq!(buf.area.height, h);
        }
    }
}
