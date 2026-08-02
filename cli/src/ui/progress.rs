//! Progress bar for tracking file uploads and long operations.

use std::io::{IsTerminal as _, Result as IoResult, Write as _, stdout};

use crossterm::{QueueableCommand as _, cursor, terminal};

use super::style as theme;

/// A single-line progress bar.
pub struct Bar {
    total: u64,
    current: u64,
    message: String,
    is_tty: bool,
    last_milestone: u8,
}

impl Bar {
    /// Creates a new progress bar with the given total and message.
    pub fn new<S>(total: u64, message: S) -> Self
    where
        S: Into<String>,
    {
        let is_tty = stdout().is_terminal();
        let message = message.into();

        if is_tty {
            best_effort(hide_cursor());
        } else {
            println!("  {message}...");
        }

        let bar = Self {
            total,
            current: 0,
            message,
            is_tty,
            last_milestone: 0,
        };
        if is_tty {
            best_effort(bar.render());
        }
        bar
    }

    /// Sets the current progress position.
    pub fn set(&mut self, pos: u64) {
        self.current = pos.min(self.total);
        if self.is_tty {
            best_effort(self.render());
            return;
        }
        let pct = if self.total > 0 {
            u8::try_from(self.current.saturating_mul(100).div_euclid(self.total)).unwrap_or(0)
        } else {
            0
        };
        let milestone = pct.div_euclid(25);
        if milestone > self.last_milestone {
            self.last_milestone = milestone;
            println!("  {pct}%");
        }
    }

    /// Increments current progress by `delta`.
    pub fn inc(&mut self, delta: u64) {
        self.set(self.current.saturating_add(delta));
    }

    /// Completes the progress bar with a final message.
    pub fn finish<S>(self, message: S)
    where
        S: Into<String>,
    {
        let msg = message.into();
        if self.is_tty {
            best_effort(render_done(&msg));
        } else {
            println!("  \u{2713} {msg}");
        }
    }

    fn render(&self) -> IoResult<()> {
        let mut out = stdout();
        let term_width = terminal::size().map_or(80_usize, |(w, _)| usize::from(w));

        let pct = if self.total > 0 {
            self.current.saturating_mul(100).div_euclid(self.total)
        } else {
            0
        };

        let size_info = format!(
            "{} / {}",
            format_bytes(self.current),
            format_bytes(self.total)
        );

        let fixed_len = [2, 1, self.message.len(), 1, 1, 4, 1, size_info.len(), 1]
            .iter()
            .sum::<usize>();
        let bar_width = term_width.saturating_sub(fixed_len).clamp(10, 40);

        let filled = if self.total > 0 {
            let bar_width_u64 = u64::try_from(bar_width).unwrap_or(0);
            usize::try_from(
                self.current
                    .saturating_mul(bar_width_u64)
                    .div_euclid(self.total),
            )
            .unwrap_or(0)
            .min(bar_width)
        } else {
            0
        };
        let empty = bar_width.saturating_sub(filled);

        let bar: String = "\u{2588}".repeat(filled);
        let rest: String = "\u{2591}".repeat(empty);

        out.queue(cursor::MoveToColumn(0))?;
        out.queue(terminal::Clear(terminal::ClearType::UntilNewLine))?;
        write!(
            out,
            "  {} {bar}{rest} {pct:>3}% ({size_info})",
            self.message,
        )?;
        out.flush()?;
        Ok(())
    }
}

/// Hides the terminal cursor before the bar renders.
fn hide_cursor() -> IoResult<()> {
    let mut out = stdout();
    out.queue(cursor::Hide)?;
    out.flush()?;
    Ok(())
}

/// Writes the completed bar line with a checkmark.
fn render_done(msg: &str) -> IoResult<()> {
    let mut out = stdout();
    out.queue(cursor::MoveToColumn(0))?;
    out.queue(terminal::Clear(terminal::ClearType::UntilNewLine))?;
    write!(out, "  {} {msg}", theme::success("\u{2713}"))?;
    out.queue(cursor::Show)?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}

/// Runs a best-effort terminal write, reporting any error.
fn best_effort(result: IoResult<()>) {
    if let Err(err) = result {
        eprintln!("progress bar write failed: {err}");
    }
}

/// Formats bytes into a human-readable size string with one decimal.
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format_bytes_with_unit(bytes, GB, 1, "GB")
    } else if bytes >= MB {
        format_bytes_with_unit(bytes, MB, 1, "MB")
    } else if bytes >= KB {
        format_bytes_with_unit(bytes, KB, 1, "KB")
    } else {
        format!("{bytes} B")
    }
}

/// Formats `bytes` as a value in `unit`, keeping `decimals` fraction digits.
fn format_bytes_with_unit(bytes: u64, unit: u64, decimals: u32, suffix: &str) -> String {
    let factor = 10_u64.pow(decimals);
    let mut whole = bytes.div_euclid(unit);
    let mut fraction = bytes
        .rem_euclid(unit)
        .wrapping_mul(factor)
        .wrapping_mul(2)
        .wrapping_add(unit)
        .div_euclid(unit.wrapping_mul(2));
    if fraction >= factor {
        fraction = fraction.rem_euclid(factor);
        whole = whole.saturating_add(1);
    }
    format!(
        "{whole}.{fraction:0width$} {suffix}",
        width = usize::try_from(decimals).unwrap_or(0)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
    }

    #[test]
    fn format_bytes_kb() {
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1024 * 1024 - 1), "1024.0 KB");
    }

    #[test]
    fn format_bytes_mb() {
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(1024 * 1024 * 512), "512.0 MB");
    }

    #[test]
    fn format_bytes_gb() {
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(format_bytes(1024 * 1024 * 1024 * 2), "2.0 GB");
    }

    #[test]
    fn set_milestone_arithmetic() {
        // ARRANGE
        let total: u64 = 1000;

        // ACT
        let positions = [0_u64, 249, 250, 499, 500, 749, 750, 999, 1000];
        let pcts: Vec<u8> = positions
            .iter()
            .map(|&pos| {
                let current = pos.min(total);
                u8::try_from(current.saturating_mul(100).div_euclid(total)).unwrap_or(0)
            })
            .collect();
        let fired: Vec<u8> = pcts
            .iter()
            .filter(|&&pct| pct.rem_euclid(25) == 0 && pct != 0)
            .copied()
            .collect();

        // ASSERT
        assert_eq!(fired, vec![25, 50, 75, 100]);
    }

    #[test]
    fn set_milestone_zero_total_never_fires() {
        // ARRANGE
        let total: u64 = 0;
        let current: u64 = 0;

        // ACT
        let pct = if total > 0 {
            u8::try_from(current.saturating_mul(100).div_euclid(total)).unwrap_or(0)
        } else {
            0
        };
        let milestone = pct.div_euclid(25);

        // ASSERT
        assert_eq!(milestone, 0);
    }
}
