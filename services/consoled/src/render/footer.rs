//! Footer drawing of the bottom info row with key hints and scroll mode.

use std::io;

use tui::screen::Screen;
use tui::style::{AnsiColor, fg};
use tui::text::{self, Line, Span};

use super::ScrollMode;

const HINT_SEGMENTS: [(&str, bool); 5] = [
    ("  ", false),
    ("\u{2191}/\u{2193}", true),
    (" \u{b7} ", false),
    ("j/k", true),
    (" scroll", false),
];

/// Draws the bottom footer with the scroll-mode indicator.
pub(super) fn draw<S: Screen>(
    screen: &mut S,
    scroll_mode: ScrollMode,
    cols: u16,
    rows: u16,
) -> io::Result<()> {
    let info_row = rows.saturating_sub(1);

    let (mode_label, esc_hint) = match scroll_mode {
        ScrollMode::Live => ("[LIVE]", ""),
        ScrollMode::Scrollback => ("[SCROLLBACK]", "  ESC live"),
    };

    let right = format!("{esc_hint}  {mode_label}  ");
    let hint_len: usize = HINT_SEGMENTS
        .iter()
        .map(|segment| text::width(segment.0))
        .sum();
    let right_len = text::width(&right);
    let padding = usize::from(cols).saturating_sub(hint_len.saturating_add(right_len));

    let mut line = Line::new(0);
    for (content, key) in HINT_SEGMENTS {
        if key {
            line.push(Span::styled(content, fg(AnsiColor::Cyan)));
        } else {
            line.push(Span::plain(content));
        }
    }
    line.push(Span::plain(" ".repeat(padding)));
    line.push(Span::styled(
        right,
        fg(match scroll_mode {
            ScrollMode::Live => AnsiColor::Green,
            ScrollMode::Scrollback => AnsiColor::Yellow,
        }),
    ));

    screen.clear_line(info_row)?;
    screen.write_line(info_row, &line)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use tui::screen::Ansi;
    use tui::size::Size;

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

    fn draw_to_bytes(scroll_mode: ScrollMode) -> Vec<u8> {
        let mut screen = Ansi::new(Vec::new(), Size::new(80, 40));
        draw(&mut screen, scroll_mode, 80, 40).unwrap();

        screen.into_inner()
    }

    #[test]
    fn footer_hint_lists_arrow_and_vim_bindings() {
        // ARRANGE
        let mode = ScrollMode::Live;

        // ACT
        let output = draw_to_bytes(mode);

        // ASSERT
        let output = String::from_utf8_lossy(&output);
        assert!(output.contains("\u{2191}/\u{2193}"));
        assert!(output.contains("j/k"));
    }

    #[test]
    fn footer_hint_and_indicator_fit_within_columns() {
        // ARRANGE
        let mode = ScrollMode::Scrollback;

        // ACT
        let output = draw_to_bytes(mode);

        // ASSERT
        let visible = strip_ansi(&String::from_utf8_lossy(&output))
            .chars()
            .count();
        assert!(visible <= 80, "footer drew {visible} visible columns");
    }
}
