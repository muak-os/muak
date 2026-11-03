//! Koci pull cache.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Base directory for the koci pull cache on the EPHEMERAL partition.
pub(crate) const DIR: &str = "/run/ephemeral/koci";

/// Subtree holding in-progress ingests, which the content store collects itself.
pub(crate) const INGEST_DIR: &str = "ingest";

// Upper bound the cache is pruned towards (10 GiB).
const BUDGET: u64 = 10 * 1024 * 1024 * 1024;

// Space below the budget that stays free (512 MiB), leaving about 9.5 GiB usable.
const FLOOR: u64 = 512 * 1024 * 1024;

// Entries younger than this are never evicted.
const MIN_AGE: Duration = Duration::from_mins(30);

/// Evicts the oldest cache entries until the store fits its byte budget.
pub(crate) fn enforce_budget() {
    enforce(
        Path::new(DIR),
        BUDGET.saturating_sub(FLOOR),
        SystemTime::now(),
    );
}

fn enforce(root: &Path, target: u64, now: SystemTime) {
    let ingest = root.join(INGEST_DIR);
    let mut entries = Vec::new();
    collect(root, &ingest, &mut entries);

    let mut used: u64 = entries.iter().map(|entry| entry.2).sum();
    if used <= target {
        return;
    }

    entries.sort_by_key(|entry| entry.0);
    let cutoff = now.checked_sub(MIN_AGE).unwrap_or(SystemTime::UNIX_EPOCH);
    let mut removed_files: u64 = 0;
    let mut removed_bytes: u64 = 0;
    for (modified, path, size) in entries {
        if used <= target || modified >= cutoff {
            break;
        }
        if fs::remove_file(&path).is_err() {
            continue;
        }
        used = used.saturating_sub(size);
        removed_files = removed_files.saturating_add(1);
        removed_bytes = removed_bytes.saturating_add(size);
    }

    prune(root, &ingest);
    if removed_files > 0 {
        kmsg::info!("Pruned {removed_files} koci cache files, freed {removed_bytes} bytes");
    }
}

fn collect(dir: &Path, ingest: &Path, entries: &mut Vec<(SystemTime, PathBuf, u64)>) {
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };

    for entry in read.filter_map(core::result::Result::ok) {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() && path == ingest {
            continue;
        }
        if file_type.is_dir() {
            collect(&path, ingest, entries);
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        entries.push((modified, path, metadata.len()));
    }
}

fn prune(dir: &Path, ingest: &Path) -> bool {
    let Ok(read) = fs::read_dir(dir) else {
        return false;
    };

    let mut empty = true;
    for entry in read.filter_map(core::result::Result::ok) {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            empty = false;
            continue;
        };
        if !file_type.is_dir() || path == ingest {
            empty = false;
            continue;
        }
        if prune(&path, ingest) {
            drop(fs::remove_dir(&path));
        } else {
            empty = false;
        }
    }

    empty
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::time::{Duration, SystemTime};

    use tempfile::TempDir;

    use super::{INGEST_DIR, enforce};

    fn write_aged(root: &TempDir, name: &str, data: &[u8], age: Duration) -> std::path::PathBuf {
        let path = root.path().join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(&path, data).expect("write file");
        let modified = SystemTime::now().checked_sub(age).expect("aged timestamp");
        let file = File::options().write(true).open(&path).expect("open file");
        file.set_modified(modified).expect("set mtime");
        drop(file);

        path
    }

    #[test]
    fn under_budget_keeps_every_entry() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let path = write_aged(
            &tmp,
            "blobs/sha256/abc",
            b"payload",
            Duration::from_hours(2),
        );

        // ACT
        enforce(tmp.path(), 64, SystemTime::now());

        // ASSERT
        assert!(path.exists());
    }

    #[test]
    fn over_budget_evicts_the_oldest_entry_first() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let oldest = write_aged(
            &tmp,
            "blobs/sha256/old",
            b"0123456789",
            Duration::from_hours(3),
        );
        let newest = write_aged(
            &tmp,
            "blobs/sha256/new",
            b"0123456789",
            Duration::from_hours(1),
        );

        // ACT: target equals a single entry, so exactly one must go
        enforce(tmp.path(), 10, SystemTime::now());

        // ASSERT
        assert!(!oldest.exists(), "the oldest entry must be evicted first");
        assert!(newest.exists(), "the newest entry must survive");
    }

    #[test]
    fn young_entries_are_never_evicted() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let old = write_aged(
            &tmp,
            "blobs/sha256/old",
            b"0123456789",
            Duration::from_hours(2),
        );
        let young = write_aged(&tmp, "blobs/sha256/young", b"0123456789", Duration::ZERO);

        // ACT
        enforce(tmp.path(), 0, SystemTime::now());

        // ASSERT
        assert!(!old.exists());
        assert!(
            young.exists(),
            "entries younger than 30 minutes must survive"
        );
    }

    #[test]
    fn ingest_subtree_is_skipped() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let staged = write_aged(
            &tmp,
            &format!("{INGEST_DIR}/abc.part"),
            b"0123456789",
            Duration::from_hours(2),
        );

        // ACT
        enforce(tmp.path(), 0, SystemTime::now());

        // ASSERT
        assert!(staged.exists(), "in-progress ingests must survive pruning");
    }

    #[test]
    fn empty_directories_are_pruned() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let blob = write_aged(
            &tmp,
            "blobs/sha256/abc",
            b"0123456789",
            Duration::from_hours(2),
        );

        // ACT
        enforce(tmp.path(), 0, SystemTime::now());

        // ASSERT
        assert!(!blob.exists());
        assert!(
            !tmp.path().join("blobs").exists(),
            "emptied directories must be removed"
        );
    }
}
