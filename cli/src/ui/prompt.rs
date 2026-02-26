//! Interactive terminal prompts.

use std::io::{IsTerminal, Write, stdin, stdout};

use anyhow::Result;
use crossterm::style::Stylize;

use super::style as theme;

/// Prompts the user to type an exact phrase to confirm a destructive action.
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
