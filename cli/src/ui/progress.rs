//! Progress bar for tracking file uploads and long operations.

use std::io::{IsTerminal, Write, stdout};

use crossterm::{QueueableCommand, cursor, terminal};

use super::style as theme;

/// A single-line progress bar.
pub struct ProgressBar {
    total: u64,
    current: u64,
    message: String,
    is_tty: bool,
    last_milestone: u8,
}

impl ProgressBar {
    /// Creates a new progress bar with the given total and message.
    pub fn new(total: u64, message: impl Into<String>) -> Self {
        let is_tty = stdout().is_terminal();
        let message = message.into();

        if is_tty {
            let mut out = stdout();
            let _ = out.queue(cursor::Hide);
            let _ = out.flush();
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
            bar.render();
        }
        bar
    }

    /// Sets the current progress position.
    pub fn set(&mut self, pos: u64) {
        self.current = pos.min(self.total);
        if self.is_tty {
            self.render();
        } else {
            let pct = if self.total > 0 {
                (self.current * 100 / self.total) as u8
            } else {
                0
            };
            let milestone = pct / 25;
            if milestone > self.last_milestone {
                self.last_milestone = milestone;
                println!("  {pct}%");
            }
        }
    }

    /// Increments current progress by `delta`.
    pub fn inc(&mut self, delta: u64) {
        self.set(self.current + delta);
    }

    /// Completes the progress bar with a final message.
    pub fn finish(self, message: impl Into<String>) {
        let msg = message.into();
        if self.is_tty {
            let mut out = stdout();
            let _ = out.queue(cursor::MoveToColumn(0));
            let _ = out.queue(terminal::Clear(terminal::ClearType::UntilNewLine));
            let _ = write!(out, "  {} {msg}", theme::success("\u{2713}"));
            let _ = out.queue(cursor::Show);
            let _ = writeln!(out);
            let _ = out.flush();
        } else {
            println!("  \u{2713} {msg}");
        }
    }

    fn render(&self) {
        let mut out = stdout();
        let term_width = terminal::size().map(|(w, _)| w).unwrap_or(80) as usize;

        let pct = if self.total > 0 {
            (self.current as f64 / self.total as f64 * 100.0) as u64
        } else {
            0
        };

        let size_info = format!(
            "{} / {}",
            format_bytes(self.current),
            format_bytes(self.total)
        );

        let fixed_len = 2 + 1 + self.message.len() + 1 + 1 + 4 + 1 + size_info.len() + 1;
        let bar_width = term_width.saturating_sub(fixed_len).clamp(10, 40);

        let filled = if self.total > 0 {
            (self.current as usize * bar_width / self.total as usize).min(bar_width)
        } else {
            0
        };
        let empty = bar_width - filled;

        let bar: String = "\u{2588}".repeat(filled);
        let rest: String = "\u{2591}".repeat(empty);

        let _ = out.queue(cursor::MoveToColumn(0));
        let _ = out.queue(terminal::Clear(terminal::ClearType::UntilNewLine));
        let _ = write!(
            out,
            "  {} {bar}{rest} {pct:>3}% ({size_info})",
            self.message,
        );
        let _ = out.flush();
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
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
        let total: u64 = 1000;
        let mut last_milestone: u8 = 0;
        let mut fired: Vec<u8> = Vec::new();

        for pos in [0u64, 249, 250, 499, 500, 749, 750, 999, 1000] {
            let current = pos.min(total);
            let pct = (current * 100 / total) as u8;
            let milestone = pct / 25;
            if milestone > last_milestone {
                last_milestone = milestone;
                fired.push(pct);
            }
        }

        assert_eq!(fired, vec![25, 50, 75, 100]);
    }

    #[test]
    fn set_milestone_zero_total_never_fires() {
        let total: u64 = 0;
        let current: u64 = 0;
        let pct = if total > 0 {
            (current * 100 / total) as u8
        } else {
            0
        };
        let milestone = pct / 25;
        assert_eq!(milestone, 0);
    }
}
