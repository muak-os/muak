//! Footer drawing of the bottom info row with key hints and scroll mode.

use std::io::Write;

use anyhow::Result;
use crossterm::cursor::MoveTo;
use crossterm::queue;
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};

use super::{ScrollMode, clear_line};

const HINT_SEGMENTS: [(&str, bool); 5] = [
    ("  ", false),
    ("\u{2191}/\u{2193}", true),
    (" \u{b7} ", false),
    ("j/k", true),
    (" scroll", false),
];
const HINT_KEY_COLOR: Color = Color::Cyan;

/// Draws the bottom footer with the scroll-mode indicator.
pub(super) fn draw(
    w: &mut impl Write,
    scroll_mode: ScrollMode,
    cols: u16,
    rows: u16,
) -> Result<()> {
    let info_row = rows.saturating_sub(1);

    let (mode_label, esc_hint) = match scroll_mode {
        ScrollMode::Live => ("[LIVE]", ""),
        ScrollMode::Scrollback => ("[SCROLLBACK]", "  ESC live"),
    };

    let right = format!("{esc_hint}  {mode_label}  ");
    let hint_len: usize = hint_visible_len();
    let right_len = right.chars().count();
    let padding = usize::from(cols).saturating_sub(hint_len.saturating_add(right_len));

    clear_line(w, info_row)?;
    queue!(w, MoveTo(0, info_row), ResetColor)?;
    for (text, key) in HINT_SEGMENTS {
        if key {
            queue!(
                w,
                SetForegroundColor(HINT_KEY_COLOR),
                Print(text),
                ResetColor
            )?;
        } else {
            queue!(w, Print(text))?;
        }
    }
    queue!(
        w,
        Print(" ".repeat(padding)),
        SetForegroundColor(match scroll_mode {
            ScrollMode::Live => Color::Green,
            ScrollMode::Scrollback => Color::Yellow,
        }),
        Print(right),
        ResetColor,
    )?;

    Ok(())
}

fn hint_visible_len() -> usize {
    HINT_SEGMENTS
        .iter()
        .map(|segment| segment.0.chars().count())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip_ansi(output: &str) -> String {
        let mut escaped = false;
        output
            .chars()
            .filter_map(|ch| {
                let (next, visible) = escape_state(escaped, ch);
                escaped = next;
                visible
            })
            .collect()
    }

    fn escape_state(escaped: bool, ch: char) -> (bool, Option<char>) {
        match (escaped, ch) {
            (true, last) if last.is_ascii_alphabetic() => (false, None),
            (true, _) | (false, '\u{1b}') => (true, None),
            (false, text) => (false, Some(text)),
        }
    }

    #[test]
    fn footer_hint_lists_arrow_and_vim_bindings() {
        // ARRANGE
        let mut buf: Vec<u8> = Vec::new();

        // ACT
        draw(&mut buf, ScrollMode::Live, 80, 40).unwrap();

        // ASSERT
        let output = String::from_utf8_lossy(&buf);
        assert!(output.contains("\u{2191}/\u{2193}"));
        assert!(output.contains("j/k"));
    }

    #[test]
    fn footer_hint_and_indicator_fit_within_columns() {
        // ARRANGE
        let mut buf: Vec<u8> = Vec::new();

        // ACT
        draw(&mut buf, ScrollMode::Scrollback, 80, 40).unwrap();

        // ASSERT
        let visible = strip_ansi(&String::from_utf8_lossy(&buf)).chars().count();
        assert!(visible <= 80, "footer drew {visible} visible columns");
    }
}
