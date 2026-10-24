//! Color policy and role palette.

use std::ffi::OsStr;

use super::semantic::Role;
use crate::style::{AnsiColor, Style, bold, bold_fg, dim, fg};

pub(super) fn should_color(
    terminal: bool,
    no_color: Option<&OsStr>,
    clicolor: Option<&OsStr>,
    clicolor_force: Option<&OsStr>,
    term: Option<&OsStr>,
) -> bool {
    if no_color.is_some_and(|value| !value.is_empty()) {
        return false;
    }

    if clicolor.is_some_and(|value| value == OsStr::new("0")) {
        return false;
    }

    if clicolor_force.is_some_and(|value| !value.is_empty() && value != OsStr::new("0")) {
        return true;
    }

    if term.is_some_and(|value| value == OsStr::new("dumb")) {
        return false;
    }

    terminal
}

pub(super) fn role_style(role: Role) -> Style {
    match role {
        Role::Success | Role::Header => bold_fg(AnsiColor::Green),
        Role::Error => bold_fg(AnsiColor::Red),
        Role::ErrorText | Role::Negative => fg(AnsiColor::Red),
        Role::Warn | Role::Highlight => fg(AnsiColor::Yellow),
        Role::Info => fg(AnsiColor::Blue),
        Role::Muted => dim(),
        Role::Accent => fg(AnsiColor::Cyan),
        Role::Label => bold(),
        Role::Positive => fg(AnsiColor::Green),
    }
}
