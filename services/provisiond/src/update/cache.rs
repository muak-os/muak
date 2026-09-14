//! Koci pull cache.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

/// Base directory for the koci pull cache on the mounted STATE partition.
pub(crate) const DIR: &str = "/run/state/.cache/koci";

/// Maximum age of a cache entry before it is removed after a successful update (3 weeks).
pub(crate) const MAX_AGE: Duration = Duration::from_hours(504);

/// Removes cache entries older than [`MAX_AGE`] and prunes emptied directories.
pub(crate) fn clean_stale() {
    let Some(cutoff) = SystemTime::now().checked_sub(MAX_AGE) else {
        return;
    };
    let removed = clean(Path::new(DIR), cutoff);
    if removed > 0 {
        kmsg::debug!("Removed {removed} stale koci cache entries");
    }
}

fn clean(dir: &Path, cutoff: SystemTime) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };

    let mut removed: usize = 0;
    for entry in entries.filter_map(core::result::Result::ok) {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            removed = removed.saturating_add(clean(&entry.path(), cutoff));
            drop(fs::remove_dir(entry.path()));
            continue;
        }
        if !is_stale(&entry, cutoff) {
            continue;
        }
        if fs::remove_file(entry.path()).is_ok() {
            removed = removed.saturating_add(1);
        }
    }

    removed
}

fn is_stale(entry: &fs::DirEntry, cutoff: SystemTime) -> bool {
    entry
        .metadata()
        .and_then(|meta| meta.modified())
        .is_ok_and(|modified| modified < cutoff)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, SystemTime};

    use tempfile::TempDir;

    use super::clean;

    #[test]
    fn stale_entries_are_removed_and_empty_dirs_pruned() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let blob = tmp.path().join("blobs/sha256/deadbeef");
        fs::create_dir_all(blob.parent().expect("blob parent")).expect("create blob dir");
        fs::write(&blob, b"blob").expect("write blob");

        // ACT: a future cutoff marks every entry as stale
        let cutoff = SystemTime::now()
            .checked_add(Duration::from_secs(60))
            .expect("future cutoff");
        let removed = clean(tmp.path(), cutoff);

        // ASSERT
        assert_eq!(removed, 1);
        assert!(!blob.exists());
        assert!(
            !tmp.path().join("blobs").exists(),
            "empty dirs must be pruned"
        );
    }

    #[test]
    fn fresh_entries_are_kept() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let r#ref = tmp.path().join("refs/ghcr.io/org/image/v1");
        fs::create_dir_all(r#ref.parent().expect("ref parent")).expect("create ref dir");
        fs::write(&r#ref, b"manifest").expect("write ref");

        // ACT: the epoch cutoff keeps everything written afterwards
        let removed = clean(tmp.path(), SystemTime::UNIX_EPOCH);

        // ASSERT
        assert_eq!(removed, 0);
        assert!(r#ref.exists());
    }

    #[test]
    fn missing_cache_dir_is_ignored() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let missing = tmp.path().join("missing");

        // ACT
        let removed = clean(&missing, SystemTime::now());

        // ASSERT
        assert_eq!(removed, 0);
        assert!(!missing.exists());
    }
}
