//! Shared live line builders.

use alloc::borrow::Cow;
use core::time::Duration;

use crate::glyphs::Glyphs;
use crate::progress;
use crate::style::{Role, Theme};
use crate::text::{self, Line, Span};

/// Returns the spinner frame at `index` for `glyphs`.
pub(crate) fn frame_at(glyphs: Glyphs, index: usize) -> &'static str {
    let modulo = glyphs.spinner.len();
    let safe = index.checked_rem(modulo).unwrap_or(0);

    glyphs.spinner.get(safe).copied().unwrap_or("")
}

/// Builds an active spinner line that fits in `width` columns.
pub(crate) fn spinner_line(
    message: &str,
    frame: usize,
    width: usize,
    theme: Theme,
    glyphs: Glyphs,
    elapsed: Option<Duration>,
) -> Line<'static> {
    let frame_text = frame_at(glyphs, frame);
    let elapsed_text = elapsed
        .filter(|elapsed| *elapsed >= Duration::from_secs(1) && width >= 80)
        .map(progress::clock);
    let elapsed_width = elapsed_text
        .as_ref()
        .map_or(0, |text| text::width(text).saturating_add(2));
    let prefix = 2_usize
        .saturating_add(text::width(frame_text))
        .saturating_add(1);
    let label = fitted(
        message,
        width.saturating_sub(prefix).saturating_sub(elapsed_width),
    );
    let mut line = Line::new(0);
    line.push(Span::plain("  "));
    line.push(Span::styled(frame_text, theme.style(Role::Accent)));
    line.push(Span::plain(" "));
    line.push(Span::plain(label));
    if let Some(elapsed) = elapsed_text {
        line.push(Span::plain("  "));
        line.push(Span::styled(elapsed, theme.style(Role::Muted)));
    }

    line
}

/// Builds a note line that fits in `width` columns.
pub(crate) fn note_line(
    message: &str,
    width: usize,
    theme: Theme,
    glyphs: Glyphs,
) -> Line<'static> {
    let prefix = 2_usize
        .saturating_add(text::width(glyphs.pending))
        .saturating_add(1);
    let mut line = Line::new(0);
    line.push(Span::plain("  "));
    line.push(Span::styled(glyphs.pending, theme.style(Role::Muted)));
    line.push(Span::plain(" "));
    line.push(Span::plain(fitted(message, width.saturating_sub(prefix))));

    line
}

/// Builds a plain message line that fits in `width` columns.
pub(crate) fn message_line(message: &str, width: usize) -> Line<'static> {
    let mut line = Line::new(0);
    line.push(Span::plain("  "));
    line.push(Span::plain(fitted(message, width.saturating_sub(2))));

    line
}

/// Builds a frozen success or failure line that fits in `width` columns.
pub(crate) fn status_line(
    message: &str,
    success: bool,
    width: usize,
    theme: Theme,
    glyphs: Glyphs,
) -> Line<'static> {
    let (mark, role) = if success {
        (glyphs.success, Role::Success)
    } else {
        (glyphs.failure, Role::Error)
    };
    let mut line = Line::new(0);
    line.push(Span::plain("  "));
    line.push(Span::styled(mark, theme.style(role)));
    line.push(Span::plain(" "));
    line.push(Span::plain(fitted(message, width.saturating_sub(4))));

    line
}

fn fitted(message: &str, width: usize) -> String {
    let sanitized = text::sanitize(Cow::Borrowed(message));

    text::truncate(&sanitized, width).to_owned()
}
