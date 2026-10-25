//! Final summary block.

use std::io;

use crate::live::renderer::write_plain;
use crate::output::{Mode, Output};
use crate::screen::{Ansi, Screen};
use crate::style::Role;
use crate::text::{self, Line, Span};

/// Final summary printed once after a command completes.
#[derive(Clone, Copy, Debug)]
pub struct Summary<'a> {
    /// Summary headline.
    pub title: &'a str,
    /// Label and value rows.
    pub rows: &'a [(&'a str, &'a str)],
    /// Machine-facing line printed instead of the summary in quiet mode.
    pub quiet: Option<&'a str>,
}

impl<'a> Summary<'a> {
    /// Creates a summary with `title` and `rows`.
    #[must_use]
    pub const fn new(title: &'a str, rows: &'a [(&'a str, &'a str)]) -> Self {
        Self {
            title,
            rows,
            quiet: None,
        }
    }

    /// Prints `value` alone when the output is quiet.
    #[must_use]
    pub const fn with_quiet(self, value: &'a str) -> Self {
        Self {
            title: self.title,
            rows: self.rows,
            quiet: Some(value),
        }
    }
}

/// Prints `summary` to standard output.
///
/// # Errors
///
/// Returns an error when writing to standard output fails.
pub fn print(summary: &Summary<'_>, output: Output) -> io::Result<()> {
    let mut screen = Ansi::new(std::io::stdout(), output.size);

    render(summary, output, &mut screen)
}

/// Renders `summary` into `screen`.
///
/// # Errors
///
/// Returns an error when writing to `screen` fails.
pub fn render<S: Screen>(summary: &Summary<'_>, output: Output, screen: &mut S) -> io::Result<()> {
    match output.mode {
        Mode::Quiet => match summary.quiet {
            Some(value) => write_plain(screen, &Line::plain(0, value)),
            None => Ok(()),
        },
        Mode::Plain => {
            let line = Line::plain(0, plain(summary));

            write_plain(screen, &line)
        }
        Mode::Rich => {
            let title = rich_title(summary, output);
            write_plain(screen, &title)?;
            let label_width = summary
                .rows
                .iter()
                .map(|&(label, _value)| text::width(label))
                .max()
                .unwrap_or(0);
            for &(label, value) in summary.rows {
                let line = rich_row(label, value, label_width, output);
                write_plain(screen, &line)?;
            }

            Ok(())
        }
    }
}

fn plain(summary: &Summary<'_>) -> String {
    if summary.rows.is_empty() {
        return format!("Success: {}", summary.title);
    }

    let rows = summary
        .rows
        .iter()
        .map(|&(label, value)| format!("{label}: {value}"))
        .collect::<Vec<_>>()
        .join("; ");

    format!("Success: {} ({rows})", summary.title)
}

fn rich_title<'a>(summary: &Summary<'a>, output: Output) -> Line<'a> {
    let mut line = Line::new(0);
    line.push(Span::plain("  "));
    line.push(Span::styled(
        output.glyphs.success,
        output.theme.style(Role::Success),
    ));
    line.push(Span::plain(" "));
    line.push(Span::plain(text::sanitize(summary.title.into())));

    line
}

fn rich_row<'a>(label: &'a str, value: &'a str, label_width: usize, output: Output) -> Line<'a> {
    let padding = label_width.saturating_sub(text::width(label));
    let mut line = Line::new(0);
    line.push(Span::plain("    "));
    line.push(Span::styled(
        text::sanitize(label.into()),
        output.theme.style(Role::Label),
    ));
    line.push(Span::plain(" ".repeat(padding.saturating_add(2))));
    line.push(Span::plain(text::sanitize(value.into())));

    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyphs::Glyphs;
    use crate::size::Size;
    use crate::style::Theme;

    fn output(mode: Mode) -> Output {
        Output {
            mode,
            theme: Theme::plain(),
            glyphs: Glyphs::plain(),
            size: Size::new(80, 25),
            verbose: false,
        }
    }

    fn rendered(summary: &Summary<'_>, output: Output) -> String {
        let mut screen = Ansi::new(Vec::new(), Size::new(80, 25));
        render(summary, output, &mut screen).expect("render summary");

        String::from_utf8_lossy(&screen.into_inner()).into_owned()
    }

    #[test]
    fn rich_summary_prints_title_and_aligned_rows() {
        // ARRANGE
        let rows = [("Tags", "v1, latest"), ("Digest", "sha256:abc")];
        let summary = Summary::new("Pushed image", &rows);

        // ACT
        let text = rendered(&summary, output(Mode::Rich));

        // ASSERT
        assert!(text.contains("[ok] Pushed image"), "rendered: {text}");
        assert!(text.contains("Tags    v1, latest"), "rendered: {text}");
        assert!(text.contains("Digest  sha256:abc"), "rendered: {text}");
    }

    #[test]
    fn plain_summary_is_a_single_line() {
        // ARRANGE
        let rows = [("Tags", "v1"), ("Digest", "sha256:abc")];
        let summary = Summary::new("Pushed image", &rows);

        // ACT
        let text = rendered(&summary, output(Mode::Plain));

        // ASSERT
        assert_eq!(
            text,
            "Success: Pushed image (Tags: v1; Digest: sha256:abc)\n"
        );
    }

    #[test]
    fn quiet_summary_prints_nothing_without_a_quiet_line() {
        // ARRANGE
        let rows = [("Digest", "sha256:abc")];
        let summary = Summary::new("Pushed image", &rows);

        // ACT
        let text = rendered(&summary, output(Mode::Quiet));

        // ASSERT
        assert!(text.is_empty(), "rendered: {text}");
    }

    #[test]
    fn quiet_summary_prints_only_its_quiet_line() {
        // ARRANGE
        let rows = [("Digest", "sha256:abc")];
        let summary = Summary::new("Pushed image", &rows).with_quiet("sha256:abc");

        // ACT
        let text = rendered(&summary, output(Mode::Quiet));

        // ASSERT
        assert_eq!(text, "sha256:abc\n");
    }

    #[test]
    fn quiet_line_is_ignored_outside_quiet_mode() {
        // ARRANGE
        let rows = [("Digest", "sha256:abc")];
        let summary = Summary::new("Pushed image", &rows).with_quiet("sha256:abc");

        // ACT
        let text = rendered(&summary, output(Mode::Plain));

        // ASSERT
        assert_eq!(text, "Success: Pushed image (Digest: sha256:abc)\n");
    }
}
