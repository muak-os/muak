//! Plain progress reporter.

use std::io::{self, Write};

use koci::progress::{Event, Reporter};
use tui::output::Output;
use tui::screen::Screen as _;
use tui::screen::ansi::Ansi;
use tui::text::Line;

/// Plain progress reporter writing one line per event to standard error.
pub(super) struct Plain {
    /// Resolved standard-error output.
    pub(super) output: Output,
}

impl Plain {
    fn emit(&self, message: &str) {
        let mut screen = Ansi::new(io::stderr(), self.output.size);
        drop(write(&mut screen, message));
    }

    fn detail(&self, message: &str) {
        if self.output.verbose {
            self.emit(message);
        }
    }
}

impl Reporter for Plain {
    fn report(&self, event: Event<'_>) {
        match event {
            Event::Started { operation, subject } => {
                self.emit(&format!("[..] {} {subject}", operation.title()));
            }
            Event::Resolved { layers, .. } => {
                self.emit(&format!("[ok] Resolved {layers} layers"));
            }
            Event::Collected { manifests, blobs } => {
                self.emit(&format!(
                    "[ok] Collected {manifests} manifests, {blobs} blobs"
                ));
            }
            Event::Packed { entries, bytes } => {
                self.emit(&format!("[ok] Packed {entries} entries ({bytes} bytes)"));
            }
            Event::IndexFetched { platforms } => {
                self.emit(&format!("[ok] Fetched index ({platforms} platforms)"));
            }
            Event::PhaseStarted { label, .. } => {
                self.emit(&format!("[..] {label}"));
            }
            Event::PhaseCompleted { label, .. } => {
                self.emit(&format!("[ok] {label}"));
            }
            Event::Progress { .. } | Event::Bytes { .. } | Event::Done { .. } => {}
            Event::ItemQueued {
                phase,
                index,
                total,
                digest,
                size,
            } => self.detail(&format!(
                "[..] {} {index}/{total} {digest} {size} bytes",
                phase.title()
            )),
            Event::ItemSkipped { digest, .. } => {
                self.detail(&format!("[ok] Skipped existing {digest}"));
            }
            Event::ItemDone { digest, size, .. } => {
                self.detail(&format!("[ok] {digest} {size} bytes"));
            }
            Event::ManifestPushed { reference } => {
                self.detail(&format!("[ok] Pushed manifest {reference}"));
            }
            Event::Verified { digest } => {
                self.emit(&format!("[ok] Verified destination digest {digest}"));
            }
            Event::Note { message } => self.detail(&format!("[..] {message}")),
        }
    }
}

fn write<W: Write>(screen: &mut Ansi<W>, message: &str) -> io::Result<()> {
    screen.write_inline(&Line::plain(0, message))?;
    screen.newline()?;
    screen.flush()
}

#[cfg(test)]
mod tests {
    use tui::size::Size;

    use super::{Ansi, write};

    #[test]
    fn write_renders_one_plain_line() {
        // ARRANGE
        let mut screen = Ansi::new(Vec::new(), Size::default());

        // ACT
        write(&mut screen, "[ok] Resolved 2 layers").expect("render the line");
        let bytes = screen.into_inner();

        // ASSERT
        assert_eq!(bytes, b"[ok] Resolved 2 layers\n");
    }
}
