//! The Chromatic Mode readout: nearest Note, sounding Pitch, Deviation, and a coarse ±50 cent
//! bar. Written as a pure function of a small view-model (`Readout`), not of `pipeline::Frame`
//! directly, so it renders — and tests — without any audio plumbing.
//!
//! Palette is blue for flat, orange for sharp, bright neutral for in tune — deliberately not
//! green and red, per ADR 0003. Hue carries direction only; magnitude comes from the bar
//! marker's position, readable with colour ignored entirely.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::pitch::IN_TUNE_TOLERANCE_CENTS;

/// How far the coarse bar reaches in either direction. The Strobe, not this bar, carries fine
/// precision past this range (ADR 0002) — this is only the coarse half.
const BAR_RANGE_CENTS: f32 = 50.0;

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
            frame.render_widget(Paragraph::new(vec![header, bar]), inner);
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
}
