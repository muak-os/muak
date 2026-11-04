//! BuildKit-compatible cache eviction policy for the koci pull cache.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use koci::pull::cache::{shared_digests, use_count};

use super::INGEST_DIR;

// Unshared entries older than this are always removed.
const KEEP_DURATION: Duration = Duration::from_hours(60 * 24);

// Hard caps applied to the limits derived from the filesystem size.
const MAX_RESERVED: u64 = 10 * 1024 * 1024 * 1024;
const MAX_USED: u64 = 100 * 1024 * 1024 * 1024;

struct Limits {
    reserved: u64,
    max_used: u64,
    min_free: u64,
}

impl Limits {
    fn from_total(total: u64) -> Self {
        Self {
            reserved: total.saturating_div(10).min(MAX_RESERVED),
            max_used: total.saturating_mul(80).saturating_div(100).min(MAX_USED),
            min_free: total.saturating_mul(20).saturating_div(100),
        }
    }
}

struct Entry {
    path: PathBuf,
    modified: SystemTime,
    size: u64,
    uses: u64,
    shared: bool,
}

struct Stats {
    used: u64,
    unshared_used: u64,
    removed_files: u64,
    removed_bytes: u64,
}

pub(crate) fn enforce(root: &Path, total: u64, free: u64) {
    let ingest = root.join(INGEST_DIR);
    let shared = shared_digests(root);
    let mut entries = Vec::new();
    collect(root, root, &ingest, &shared, &mut entries);

    let limits = Limits::from_total(total);
    let cutoff = SystemTime::now()
        .checked_sub(KEEP_DURATION)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut stats = Stats {
        used: entries.iter().map(|entry| entry.size).sum(),
        unshared_used: entries
            .iter()
            .filter(|entry| !entry.shared)
            .map(|entry| entry.size)
            .sum(),
        removed_files: 0,
        removed_bytes: 0,
    };

    // (a) expire unshared entries that no indexed manifest references any more.
    entries = sweep(entries, &mut stats, |entry, _| {
        !entry.shared && entry.modified < cutoff
    });
    let mut fired = stats.removed_files > 0;

    entries.sort_by_key(|entry| (entry.modified, entry.uses));

    // (b) keep unshared entries under their own budget.
    let keep = keep_bytes(stats.unshared_used, free, &limits);
    if stats.unshared_used > keep {
        fired = true;
        entries = sweep(entries, &mut stats, |entry, stats| {
            !entry.shared && stats.unshared_used >= keep
        });
    }

    // (c) keep the whole cache under the global budget.
    let keep = keep_bytes(stats.used, free, &limits);
    if stats.used > keep {
        fired = true;
        drop(sweep(entries, &mut stats, |_, stats| stats.used >= keep));
    }

    remove_empty(root, &ingest);

    kmsg::info!(
        "koci cache GC: used={} free={free} removed={} files ({} bytes) policy_fired={fired}",
        stats.used,
        stats.removed_files,
        stats.removed_bytes
    );
}

fn keep_bytes(used: u64, free: u64, limits: &Limits) -> u64 {
    let mut keep = limits.max_used;
    if free < limits.min_free {
        let deficit = limits.min_free.saturating_sub(free);
        keep = keep.min(used.saturating_sub(deficit));
    }

    keep.max(limits.reserved)
}

fn sweep(
    entries: Vec<Entry>,
    stats: &mut Stats,
    mut should_evict: impl FnMut(&Entry, &Stats) -> bool,
) -> Vec<Entry> {
    let mut kept = Vec::with_capacity(entries.len());
    for entry in entries {
        if should_evict(&entry, stats) && remove(&entry, stats) {
            continue;
        }
        kept.push(entry);
    }

    kept
}

fn remove(entry: &Entry, stats: &mut Stats) -> bool {
    if fs::remove_file(&entry.path).is_err() {
        return false;
    }
    stats.used = stats.used.saturating_sub(entry.size);
    if !entry.shared {
        stats.unshared_used = stats.unshared_used.saturating_sub(entry.size);
    }
    stats.removed_files = stats.removed_files.saturating_add(1);
    stats.removed_bytes = stats.removed_bytes.saturating_add(entry.size);

    true
}

fn collect(
    root: &Path,
    dir: &Path,
    ingest: &Path,
    shared: &HashSet<String>,
    entries: &mut Vec<Entry>,
) {
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };

    for entry in read.filter_map(Result::ok) {
        let path = entry.path();
        if path == ingest {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            collect(root, &path, ingest, shared, entries);
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        let shared = is_shared(root, &path, shared);
        let uses = use_count(&path);
        entries.push(Entry {
            path,
            modified,
            size: metadata.len(),
            uses,
            shared,
        });
    }
}

fn is_shared(root: &Path, path: &Path, shared: &HashSet<String>) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return true;
    };
    let Some(component) = relative.components().next() else {
        return true;
    };
    match component.as_os_str().to_str() {
        Some("refs" | "index") => true,
        Some("blobs") => path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|hash| shared.contains(&format!("sha256:{hash}"))),
        _ => false,
    }
}

fn remove_empty(dir: &Path, ingest: &Path) -> bool {
    let Ok(read) = fs::read_dir(dir) else {
        return false;
    };

    let mut empty = true;
    for entry in read.filter_map(Result::ok) {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            empty = false;
            continue;
        };
        if !file_type.is_dir() || path == ingest {
            empty = false;
            continue;
        }
        if remove_empty(&path, ingest) {
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
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    use koci::pull::cache::bump_use_count;
    use tempfile::TempDir;

    use super::{Limits, enforce, keep_bytes};
    use crate::cache::INGEST_DIR;

    fn write(root: &Path, relative: &str, data: &[u8]) -> PathBuf {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(&path, data).expect("write file");

        path
    }

    fn set_modified(path: &Path, modified: SystemTime) {
        let file = File::options().write(true).open(path).expect("open file");
        file.set_modified(modified).expect("set mtime");
    }

    fn aged(duration: Duration) -> SystemTime {
        SystemTime::now()
            .checked_sub(duration)
            .expect("aged timestamp")
    }

    fn ingest_path(name: &str) -> String {
        format!("{INGEST_DIR}/{name}")
    }

    #[test]
    fn limits_follow_buildkit_defaults() {
        // ARRANGE
        let gib = 1024 * 1024 * 1024;

        // ACT
        let limits = Limits::from_total(10 * gib);

        // ASSERT
        assert_eq!(limits.reserved, gib);
        assert_eq!(limits.max_used, 8 * gib);
        assert_eq!(limits.min_free, 2 * gib);
    }

    #[test]
    fn keep_bytes_shrinks_when_free_space_is_low() {
        // ARRANGE
        let gib = 1024 * 1024 * 1024;
        let limits = Limits::from_total(10 * gib);

        // ACT
        let spacious = keep_bytes(4 * gib, 5 * gib, &limits);
        let tight = keep_bytes(6 * gib, gib, &limits);
        let floored = keep_bytes(0, 0, &limits);

        // ASSERT
        assert_eq!(spacious, 8 * gib);
        assert_eq!(tight, 5 * gib, "deficit of 1 GiB leaves 5 GiB");
        assert_eq!(floored, gib, "keep never drops below the reserved slice");
    }

    #[test]
    fn unshared_entries_older_than_sixty_days_expire() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let root = tmp.path();
        let expired = write(root, "blobs/sha256/expired", &[0; 10]);
        let young = write(root, "blobs/sha256/young", &[0; 10]);
        let shared = write(root, "refs/registry/name/tag", &[0; 10]);
        set_modified(&expired, aged(Duration::from_hours(61 * 24)));
        set_modified(&young, aged(Duration::from_hours(59 * 24)));
        set_modified(&shared, aged(Duration::from_hours(61 * 24)));

        // ACT
        enforce(root, 100, 100);

        // ASSERT
        assert!(!expired.exists());
        assert!(young.exists());
        assert!(shared.exists(), "shared entries are not aged out");
    }

    #[test]
    fn unshared_entries_are_evicted_before_shared_ones() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let root = tmp.path();
        let older_unshared = write(root, "blobs/sha256/old", &[0; 50]);
        let newer_unshared = write(root, "blobs/sha256/new", &[0; 50]);
        let shared = write(root, "refs/registry/name/tag", &[0; 10]);
        set_modified(&older_unshared, aged(Duration::from_hours(3)));
        set_modified(&newer_unshared, aged(Duration::from_hours(1)));
        set_modified(&shared, aged(Duration::from_hours(2)));

        // ACT
        enforce(root, 100, 50);

        // ASSERT
        assert!(!older_unshared.exists(), "the oldest unshared entry goes");
        assert!(newer_unshared.exists(), "unshared entries fit the budget");
        assert!(shared.exists(), "shared entries are never evicted first");
    }

    #[test]
    fn equal_age_eviction_prefers_the_least_used_entry() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let root = tmp.path();
        let instant = aged(Duration::from_hours(2));
        let cold = write(root, "blobs/sha256/cold", &[0; 50]);
        let hot = write(root, "blobs/sha256/hot", &[0; 50]);
        set_modified(&cold, instant);
        set_modified(&hot, instant);
        for _ in 0..3_u64 {
            bump_use_count(&hot);
        }

        // ACT
        enforce(root, 100, 50);

        // ASSERT
        assert!(!cold.exists(), "the least used entry is evicted first");
        assert!(hot.exists());
    }

    #[test]
    fn ingest_subtree_is_skipped() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let root = tmp.path();
        let staged = write(root, &ingest_path("abc.part"), &[0; 200]);
        let blob = write(root, "blobs/sha256/abc", &[0; 50]);
        set_modified(&staged, aged(Duration::from_hours(61 * 24)));
        set_modified(&blob, aged(Duration::from_hours(2)));

        // ACT
        enforce(root, 10, 0);

        // ASSERT
        assert!(staged.exists(), "in-progress ingests are never evicted");
        assert!(!blob.exists());
    }

    #[test]
    fn emptied_directories_are_pruned() {
        // ARRANGE
        let tmp = TempDir::new().expect("temp dir");
        let root = tmp.path();
        let blob = write(root, "blobs/sha256/abc", &[0; 50]);
        let staged = write(root, &ingest_path("keep.part"), b"data");
        set_modified(&blob, aged(Duration::from_hours(2)));

        // ACT
        enforce(root, 10, 0);

        // ASSERT
        assert!(!blob.exists());
        assert!(!root.join("blobs").exists(), "emptied dirs are removed");
        assert!(staged.exists());
        assert!(root.join(INGEST_DIR).exists(), "ingest is never pruned");
    }
}
