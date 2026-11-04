//! Durable records for failed update preparation.

use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Error;
use config::system::CONFIG_PATH;

use crate::journal::{self, ChangeKind, Entry};

// Maximum bytes of the line written to the kernel log.
const SUMMARY_MAX_BYTES: usize = 400;

// Disambiguates attempts that fail within the same second.
static ATTEMPT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

// Records a failed update preparation.
pub(super) fn record(version: &str, author: &str, error: &Error) {
    let line = format!("Update preparation for {version} failed: {error}");
    kmsg::error!("{}", summary(&line));

    let config = match fs::read_to_string(CONFIG_PATH) {
        Ok(contents) => contents,
        Err(e) => {
            kmsg::error!("Skipping failed-update journal entry: cannot read config: {e}");
            return;
        }
    };

    let update_id = attempt_id();
    let entry = Entry::new(&update_id, author, ChangeKind::PrepareFailed)
        .failed(version, &format!("{error:#}"));

    if let Err(e) = journal::append(&entry, &config) {
        kmsg::error!("Skipping failed-update journal entry {update_id}: {e}");
    }
}

fn attempt_id() -> String {
    let sequence = ATTEMPT_SEQUENCE.fetch_add(1, Ordering::Relaxed);

    format!("prepare-{}-{sequence}", journal::now())
}

fn summary(message: &str) -> &str {
    if message.len() <= SUMMARY_MAX_BYTES {
        return message;
    }

    let mut end = SUMMARY_MAX_BYTES;
    while !message.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }

    message.get(..end).unwrap_or(message)
}

#[cfg(test)]
mod tests {
    use super::{attempt_id, summary};

    #[test]
    fn short_summary_passes_through() {
        // ARRANGE
        let message = "fetch failed: connection refused";

        // ACT
        let result = summary(message);

        // ASSERT
        assert_eq!(result, message);
    }

    #[test]
    fn attempt_ids_are_unique() {
        // ARRANGE / ACT
        let first = attempt_id();
        let second = attempt_id();

        // ASSERT
        assert!(first.starts_with("prepare-"), "{first}");
        assert!(second.starts_with("prepare-"), "{second}");
        assert_ne!(first, second);
    }

    #[test]
    fn long_version_is_bounded_on_the_whole_line() {
        // ARRANGE
        let version = format!("v1.0.0-{}", "a".repeat(500));
        let line = format!("Update preparation for {version} failed: fetch failed");

        // ACT
        let result = summary(&line);

        // ASSERT
        assert_eq!(result.len(), super::SUMMARY_MAX_BYTES);
        assert!(line.starts_with(result));
    }

    #[test]
    fn long_summary_is_truncated_on_char_boundary() {
        // ARRANGE
        let message = "\u{20ac}".repeat(200);
        assert!(message.len() > super::SUMMARY_MAX_BYTES);

        // ACT
        let result = summary(&message);

        // ASSERT
        assert_eq!(result.len(), super::SUMMARY_MAX_BYTES.saturating_sub(1));
        assert!(message.is_char_boundary(result.len()));
        assert!(message.starts_with(result));
    }
}
