#![cfg(feature = "live")]

//! Integration tests for the live byte tracking step display.

pub mod common;

#[cfg(test)]
mod tests {
    use std::sync::mpsc::Receiver;

    use tui::live::Options;
    use tui::live::steps::Steps;
    use tui::live::tracker::Tracked;
    use tui::size::Size;
    use tui::style::Theme;

    use crate::common::recording::{Recorder, collect};

    fn tracked(verbose: bool) -> (Tracked, Steps, Receiver<String>) {
        let (screen, rx) = Recorder::new(Size::new(80, 24));
        let steps = Steps::new(screen, Options::new(true, Theme::plain())).expect("steps");
        let tracked = Tracked::new(steps.handle(), verbose);

        (tracked, steps, rx)
    }

    fn output(rx: &Receiver<String>, steps: Steps) -> String {
        steps.finish().expect("finish");

        collect(rx)
    }

    #[test]
    fn bytes_aggregate_across_interleaved_items() {
        // ARRANGE
        let (tracked, steps, rx) = tracked(false);

        // ACT
        tracked.start_bytes("transfer", 4096);
        tracked.set_size(1, 1024);
        tracked.set_size(2, 1024);
        tracked.bytes(Some(1), 1024, 1024);
        tracked.bytes(Some(2), 512, 1024);
        tracked.bytes(Some(1), 1024, 1024);
        tracked.bytes(Some(2), 1024, 1024);
        let text = output(&rx, steps);

        // ASSERT
        assert!(text.contains("1 KiB / 4 KiB"), "output: {text}");
        assert!(text.contains("1.5 KiB / 4 KiB"), "output: {text}");
        assert!(text.contains("2 KiB / 4 KiB"), "output: {text}");
    }

    #[test]
    fn item_done_finishes_the_indexed_item() {
        // ARRANGE
        let (tracked, steps, rx) = tracked(false);

        // ACT
        tracked.start_bytes("download", 2048);
        tracked.set_size(1, 1024);
        tracked.set_size(2, 1024);
        tracked.item_done(1);
        tracked.item_done(2);
        let text = output(&rx, steps);

        // ASSERT
        assert!(text.contains("1 KiB / 2 KiB"), "output: {text}");
        assert!(text.contains("2 KiB / 2 KiB"), "output: {text}");
        assert!(text.contains("100%"), "output: {text}");
    }

    #[test]
    fn sized_artifacts_labels_the_step_by_count() {
        // ARRANGE
        let (tracked, steps, rx) = tracked(false);

        // ACT
        tracked.sized_artifacts(1, Some(10));
        tracked.sized_artifacts(3, None);
        let text = output(&rx, steps);

        // ASSERT
        assert!(text.contains("Building 1 artifact"), "output: {text}");
        assert!(text.contains("Building 3 artifacts"), "output: {text}");
    }

    #[test]
    fn sized_resets_previous_byte_samples() {
        // ARRANGE
        let (tracked, steps, rx) = tracked(false);

        // ACT
        tracked.start_bytes("first", 4096);
        tracked.bytes(Some(1), 4096, 4096);
        tracked.sized("second", Some(8192));
        tracked.bytes(Some(2), 256, 256);
        let text = output(&rx, steps);

        // ASSERT
        assert!(text.contains("256 B / 8 KiB"), "output: {text}");
        assert!(!text.contains("4.2 KiB"), "output: {text}");
    }

    #[test]
    fn start_items_resets_previous_byte_samples() {
        // ARRANGE
        let (tracked, steps, rx) = tracked(false);

        // ACT
        tracked.start_bytes("first", 4096);
        tracked.bytes(Some(1), 4096, 4096);
        tracked.start_items("second", 10);
        tracked.bytes(Some(2), 5, 5);
        let text = output(&rx, steps);

        // ASSERT
        assert!(text.contains("5/10"), "output: {text}");
        assert!(!text.contains("10/10"), "output: {text}");
    }

    #[test]
    fn sized_without_bytes_starts_a_spinner() {
        // ARRANGE
        let (tracked, steps, rx) = tracked(false);

        // ACT
        tracked.sized("building", None);
        let text = output(&rx, steps);

        // ASSERT
        assert!(text.contains("building"), "output: {text}");
        assert!(!text.contains('%'), "output: {text}");
    }

    #[test]
    fn unindexed_bytes_clamp_progress_to_total() {
        // ARRANGE
        let (tracked, steps, rx) = tracked(false);

        // ACT
        tracked.start_items("items", 10);
        tracked.bytes(None, 50, 10);
        let text = output(&rx, steps);

        // ASSERT
        assert!(text.contains("10/10"), "output: {text}");
        assert!(text.contains("100%"), "output: {text}");
    }

    #[test]
    fn quiet_notes_hide_detail_and_verbose_notes_show_it() {
        // ARRANGE
        let (quiet, quiet_steps, quiet_rx) = tracked(false);
        let (verbose, verbose_steps, verbose_rx) = tracked(true);

        // ACT
        quiet.note("always shown");
        quiet.note_if_verbose("hidden digest");
        quiet.note_detail(Some("kernel"), "hidden detail", true);
        quiet.note_detail(None, "shared detail", false);
        verbose.note_if_verbose("visible digest");
        verbose.note_detail(Some("kernel"), "visible detail", true);
        let quiet_text = output(&quiet_rx, quiet_steps);
        let verbose_text = output(&verbose_rx, verbose_steps);

        // ASSERT
        assert!(quiet_text.contains("always shown"), "output: {quiet_text}");
        assert!(
            !quiet_text.contains("hidden digest"),
            "output: {quiet_text}"
        );
        assert!(
            !quiet_text.contains("hidden detail"),
            "output: {quiet_text}"
        );
        assert!(quiet_text.contains("shared detail"), "output: {quiet_text}");
        assert!(
            verbose_text.contains("visible digest"),
            "output: {verbose_text}"
        );
        assert!(
            verbose_text.contains("kernel: visible detail"),
            "output: {verbose_text}"
        );
    }

    #[test]
    fn forced_node_state_shows_without_verbose() {
        // ARRANGE
        let (tracked, steps, rx) = tracked(false);

        // ACT
        tracked.note_state("iso", "failed", true);
        tracked.note_state("iso", "started", false);
        let text = output(&rx, steps);

        // ASSERT
        assert!(text.contains("iso: failed"), "output: {text}");
        assert!(!text.contains("iso: started"), "output: {text}");
    }
}
