//! Terminal progress adapter.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tui::live::StepsHandle;

use crate::progress::Progress;

/// Pull progress reporter rendered with `tui` steps.
pub struct Reporter {
    steps: StepsHandle,
    downloaded: AtomicU64,
    scanned: AtomicU64,
    extracted: AtomicU64,
    scan_started: AtomicBool,
}

impl Reporter {
    /// Creates a progress adapter over `steps`.
    #[must_use]
    pub fn new(steps: StepsHandle) -> Self {
        Self {
            steps,
            downloaded: AtomicU64::new(0),
            scanned: AtomicU64::new(0),
            extracted: AtomicU64::new(0),
            scan_started: AtomicBool::new(false),
        }
    }

    fn finish_previous(&self, total: u64) {
        if self.scan_started.load(Ordering::Relaxed) {
            let scanned = self.scanned.load(Ordering::Relaxed);
            self.steps
                .complete(format!("Scanned {scanned}/{total} layers"));
        } else {
            let downloaded = self.downloaded.load(Ordering::Relaxed);
            self.steps
                .complete(format!("Downloaded {downloaded}/{total} layers"));
        }
    }
}

impl Progress for Reporter {
    fn pulling(&self, reference: &str, arch: &str) {
        self.downloaded.store(0, Ordering::Relaxed);
        self.scanned.store(0, Ordering::Relaxed);
        self.extracted.store(0, Ordering::Relaxed);
        self.scan_started.store(false, Ordering::Relaxed);
        self.steps
            .start(format!("Pulling image {reference} for {arch}"));
    }

    fn resolved(&self, layers: usize) {
        let total = count(layers);
        self.steps.complete(format!("Resolved {layers} layer(s)"));
        self.steps
            .start_progress(format!("Downloading layers 0/{total}"), total);
    }

    fn layer_scanning(&self, index: usize, total: usize, digest: &str) {
        let total = count(total);
        if !self.scan_started.swap(true, Ordering::Relaxed) {
            let downloaded = self.downloaded.load(Ordering::Relaxed);
            self.steps
                .complete(format!("Downloaded {downloaded}/{total} layers"));
            self.steps
                .start_progress(format!("Scanning layers 0/{total}"), total);
        }

        let scanned = self
            .scanned
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        self.steps
            .set_label(format!("Scanning layers {index}/{total}: {digest}"));
        self.steps.progress(scanned.min(total));
    }

    fn layer_downloaded(&self, index: usize, total: usize, digest: &str) {
        let total = count(total);
        let downloaded = self
            .downloaded
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        self.steps
            .set_label(format!("Downloading layers {index}/{total}: {digest}"));
        self.steps.progress(downloaded.min(total));
    }

    fn layer_extracting(&self, index: usize, total: usize, digest: &str) {
        let total = count(total);
        if self.extracted.load(Ordering::Relaxed) == 0 {
            self.finish_previous(total);
            self.steps
                .start_progress(format!("Extracting layers 0/{total}"), total);
        }

        let extracted = self
            .extracted
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        self.steps
            .set_label(format!("Extracting layers {index}/{total}: {digest}"));
        self.steps.progress(extracted.min(total));
    }
}

fn count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

    use tui::live::{Options, Steps};
    use tui::screen::Screen;
    use tui::size::Size;
    use tui::style::Theme;
    use tui::text::Span;

    use super::*;

    struct RecordingScreen {
        tx: SyncSender<String>,
        size: Size,
    }

    impl RecordingScreen {
        fn new(size: Size) -> (Self, Receiver<String>) {
            let (tx, rx) = sync_channel(1024);

            (Self { tx, size }, rx)
        }

        fn send(&self, text: impl Into<String>) {
            let _send_result = self.tx.try_send(text.into());
        }
    }

    impl Screen for RecordingScreen {
        fn size(&self) -> Size {
            self.size
        }

        fn move_to(&mut self, _col: u16, _row: u16) -> io::Result<()> {
            Ok(())
        }

        fn move_to_column(&mut self, _col: u16) -> io::Result<()> {
            Ok(())
        }

        fn clear_until_newline(&mut self) -> io::Result<()> {
            Ok(())
        }

        fn write_span(&mut self, span: &Span<'_>) -> io::Result<()> {
            self.send(span.text());

            Ok(())
        }

        fn set_scroll_region(&mut self, _top: u16, _bottom: u16) -> io::Result<()> {
            Ok(())
        }

        fn reset_scroll_region(&mut self) -> io::Result<()> {
            Ok(())
        }

        fn hide_cursor(&mut self) -> io::Result<()> {
            Ok(())
        }

        fn show_cursor(&mut self) -> io::Result<()> {
            Ok(())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn maps_pull_events_to_ordered_phases() {
        // ARRANGE
        let (screen, rx) = RecordingScreen::new(Size::new(80, 24));
        let options = Options {
            tty: true,
            theme: Theme::plain(),
        };
        let steps = Steps::new(screen, options).unwrap();
        let progress = Reporter::new(steps.handle());

        // ACT
        progress.pulling("repo:test", "x86_64");
        progress.resolved(2);
        progress.layer_download_queued(1, 2, "aaa");
        progress.layer_downloaded(1, 2, "aaa");
        progress.layer_downloaded(2, 2, "bbb");
        progress.layer_scanning(1, 2, "aaa");
        progress.layer_scanning(2, 2, "bbb");
        progress.layer_extracting(1, 2, "aaa");
        progress.layer_extracting(2, 2, "bbb");
        steps.finish().unwrap();
        let output: String = rx.iter().collect();

        // ASSERT
        assert!(output.contains("Pulling image"), "output: {output}");
        assert!(output.contains("Resolved 2 layer"), "output: {output}");
        assert!(output.contains("Downloading layers"), "output: {output}");
        assert!(output.contains("Scanning layers"), "output: {output}");
        assert!(output.contains("Extracting layers"), "output: {output}");
    }
}
