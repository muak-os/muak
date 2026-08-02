//! Interactive terminal prompts.

use std::io::{IsTerminal as _, Write as _, stdin, stdout};

use anyhow::Result;
use crossterm::style::Stylize as _;

use super::style as theme;

/// Prompts the user to type an exact phrase to confirm a destructive action.
///
/// # Errors
///
/// Returns an error if reading from stdin fails.
pub fn confirm_phrase(message: &str, phrase: &str) -> Result<bool> {
    if !stdout().is_terminal() {
        return Ok(false);
    }

    print!("{message} Type '{}' to confirm: ", phrase.bold());
    stdout().flush()?;

    let mut input = String::new();
    stdin().read_line(&mut input)?;

    Ok(input.trim() == phrase)
}

/// Prompts for a simple yes/no confirmation.
///
/// # Errors
///
/// Returns an error if reading from stdin fails.
pub fn confirm(message: &str) -> Result<bool> {
    if !stdout().is_terminal() {
        return Ok(false);
    }

    print!("{message} {} ", theme::muted("[y/N]"));
    stdout().flush()?;

    let mut input = String::new();
    stdin().read_line(&mut input)?;

    Ok(matches!(input.trim().to_lowercase().as_str(), "y" | "yes"))
}
