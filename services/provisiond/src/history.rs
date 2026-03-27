//! Config change history: recording, pruning, and querying.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::update::snapshot;

/// Directory holding config history entries.
const HISTORY_DIR: &str = "/run/state/history";

/// Maximum number of config history entries to retain.
const HISTORY_MAX_ENTRIES: usize = 1000;

/// The kind of operation that produced a config change.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChangeKind {
    Install,
    Update,
    Rollback,
}

impl std::fmt::Display for ChangeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChangeKind::Install => write!(f, "install"),
            ChangeKind::Update => write!(f, "update"),
            ChangeKind::Rollback => write!(f, "rollback"),
        }
    }
}

/// Metadata for a single config history entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub timestamp: i64,
    pub update_id: String,
    pub author: String,
    pub change_kind: ChangeKind,
}

/// Records a config change, then prunes old entries.
pub fn record(update_id: &str, author: &str, kind: ChangeKind, new_config: &str) -> Result<()> {
    let dir = Path::new(HISTORY_DIR);
    std::fs::create_dir_all(dir).context("Failed to create history dir")?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let entry = HistoryEntry {
        timestamp,
        update_id: update_id.to_string(),
        author: author.to_string(),
        change_kind: kind,
    };

    let stem = stem(timestamp, update_id);
    let json = serde_json::to_string_pretty(&entry)?;

    std::fs::write(dir.join(format!("{}.json", stem)), json)
        .context("Failed to write history metadata")?;
    std::fs::write(
        dir.join(format!("{}.{}", stem, config::CONFIG_EXTENSION)),
        new_config,
    )
    .context("Failed to write history config snapshot")?;

    prune(dir, HISTORY_MAX_ENTRIES)?;

    Ok(())
}

/// Returns all history entries sorted from newest to oldest, capped at `limit`.
pub fn list(limit: usize) -> Result<Vec<HistoryEntry>> {
    let dir = Path::new(HISTORY_DIR);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = read_all_entries(dir)?;
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    entries.truncate(limit);
    Ok(entries)
}

/// Returns the config at `update_id`, or the live config if `update_id` is empty.
pub fn config(update_id: &str) -> Result<String> {
    if update_id.is_empty() {
        return std::fs::read_to_string(config::CONFIG_PATH)
            .context("Failed to read current config");
    }

    let dir = Path::new(HISTORY_DIR);
    let path = snapshot::find(dir, update_id)?;
    std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read history snapshot for {}", update_id))
}

/// Removes the oldest entries beyond `max_entries`, deleting both
fn prune(dir: &Path, max_entries: usize) -> Result<()> {
    let mut stems = read_all_stems(dir)?;
    if stems.len() <= max_entries {
        return Ok(());
    }

    stems.sort_unstable();
    let to_delete = stems.len() - max_entries;

    for stem in stems.iter().take(to_delete) {
        for ext in &["json", config::CONFIG_EXTENSION] {
            let path = dir.join(format!("{}.{}", stem, ext));
            if let Err(e) = std::fs::remove_file(&path) {
                eprintln!("Failed to prune history file {:?}: {}", path, e);
            }
        }
    }

    Ok(())
}

fn read_all_entries(dir: &Path) -> Result<Vec<HistoryEntry>> {
    std::fs::read_dir(dir)
        .context("Failed to read history dir")?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
        .map(|e| {
            let contents = std::fs::read_to_string(e.path())
                .with_context(|| format!("Failed to read {:?}", e.path()))?;
            serde_json::from_str(&contents)
                .with_context(|| format!("Failed to parse {:?}", e.path()))
        })
        .collect()
}

fn read_all_stems(dir: &Path) -> Result<Vec<String>> {
    let stems: Vec<String> = std::fs::read_dir(dir)
        .context("Failed to read history dir")?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
        .filter_map(|e| {
            e.path()
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_owned)
        })
        .collect();
    Ok(stems)
}

fn stem(timestamp: i64, update_id: &str) -> String {
    format!("{:020}-{}", timestamp, update_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stem_is_sortable() {
        // ARRANGE
        let a = stem(1000, "update-1000");
        let b = stem(2000, "update-2000");

        // ACT & ASSERT
        assert!(a < b);
    }

    #[test]
    fn change_kind_display() {
        // ACT & ASSERT
        assert_eq!(ChangeKind::Install.to_string(), "install");
        assert_eq!(ChangeKind::Update.to_string(), "update");
        assert_eq!(ChangeKind::Rollback.to_string(), "rollback");
    }

    #[test]
    fn stem_zero_pads_timestamp_to_twenty_digits() {
        // ARRANGE
        let ts = 1i64;
        let id = "abc";

        // ACT
        let s = stem(ts, id);

        // ASSERT
        assert_eq!(&s[..20], "00000000000000000001");
        assert!(s.ends_with("-abc"));
    }

    #[test]
    fn change_kind_roundtrips_through_serde() {
        // ARRANGE
        let kinds = [
            ChangeKind::Install,
            ChangeKind::Update,
            ChangeKind::Rollback,
        ];

        for kind in &kinds {
            // ACT
            let json = serde_json::to_string(kind).expect("serialize");
            let decoded: ChangeKind = serde_json::from_str(&json).expect("deserialize");

            // ASSERT
            assert_eq!(*kind, decoded);
        }
    }

    #[test]
    fn history_entry_roundtrips_through_serde() {
        // ARRANGE
        let entry = HistoryEntry {
            timestamp: 1_700_000_000,
            update_id: "update-1700000000".to_string(),
            author: "alice".to_string(),
            change_kind: ChangeKind::Update,
        };

        // ACT
        let json = serde_json::to_string(&entry).expect("serialize");
        let decoded: HistoryEntry = serde_json::from_str(&json).expect("deserialize");

        // ASSERT
        assert_eq!(decoded.timestamp, entry.timestamp);
        assert_eq!(decoded.update_id, entry.update_id);
        assert_eq!(decoded.author, entry.author);
        assert_eq!(decoded.change_kind, entry.change_kind);
    }

    #[test]
    fn read_all_stems_returns_only_json_stems() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("aaa.json"), "{}").unwrap();
        std::fs::write(dir.path().join("bbb.toml"), "x = 1").unwrap();
        std::fs::write(dir.path().join("ccc.json"), "{}").unwrap();

        // ACT
        let mut stems = read_all_stems(dir.path()).expect("read_all_stems");
        stems.sort();

        // ASSERT
        assert_eq!(stems, vec!["aaa", "ccc"]);
    }

    #[test]
    fn read_all_entries_deserializes_valid_json_files() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        let entry = HistoryEntry {
            timestamp: 42,
            update_id: "update-42".to_string(),
            author: "bob".to_string(),
            change_kind: ChangeKind::Install,
        };
        let json = serde_json::to_string(&entry).expect("serialize");
        std::fs::write(dir.path().join("entry.json"), &json).unwrap();

        // ACT
        let entries = read_all_entries(dir.path()).expect("read_all_entries");

        // ASSERT
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].update_id, "update-42");
        assert_eq!(entries[0].author, "bob");
        assert_eq!(entries[0].change_kind, ChangeKind::Install);
    }

    #[test]
    fn prune_removes_oldest_entries_beyond_max() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        for i in 0..5u64 {
            let name = stem(i as i64, &format!("update-{}", i));
            std::fs::write(dir.path().join(format!("{}.json", name)), "{}").unwrap();
            std::fs::write(dir.path().join(format!("{}.toml", name)), "").unwrap();
        }

        // ACT
        prune(dir.path(), 3).expect("prune");

        // ASSERT
        let mut remaining = read_all_stems(dir.path()).expect("read_all_stems");
        remaining.sort();
        assert_eq!(remaining.len(), 3);
        for stem_str in &remaining {
            let ts_part: u64 = stem_str[..20].trim_start_matches('0').parse().unwrap_or(0);
            assert!(ts_part >= 2, "expected only ts>=2, got {}", ts_part);
        }
    }

    #[test]
    fn prune_does_nothing_when_below_max() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        for i in 0..3u64 {
            let name = stem(i as i64, &format!("update-{}", i));
            std::fs::write(dir.path().join(format!("{}.json", name)), "{}").unwrap();
        }

        // ACT
        prune(dir.path(), 10).expect("prune");

        // ASSERT
        let stems = read_all_stems(dir.path()).expect("read_all_stems");
        assert_eq!(stems.len(), 3);
    }
}
