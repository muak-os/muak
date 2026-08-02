//! Config change history: recording, pruning, and querying.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
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

impl core::fmt::Display for ChangeKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
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

    let timestamp = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
    .unwrap_or(0);

    let entry = HistoryEntry {
        timestamp,
        update_id: update_id.to_owned(),
        author: author.to_owned(),
        change_kind: kind,
    };

    let stem = stem(timestamp, update_id);
    let json = serde_json::to_string_pretty(&entry)?;

    std::fs::write(dir.join(format!("{stem}.json")), json)
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
    entries.sort_by_key(|entry| core::cmp::Reverse(entry.timestamp));
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
        .with_context(|| format!("Failed to read history snapshot for {update_id}"))
}

/// Removes the oldest entries beyond `max_entries`, deleting both.
fn prune(dir: &Path, max_entries: usize) -> Result<()> {
    let mut stems = read_all_stems(dir)?;
    if stems.len() <= max_entries {
        return Ok(());
    }

    stems.sort_unstable();
    let to_delete = stems.len().saturating_sub(max_entries);

    for stem in stems.iter().take(to_delete) {
        for ext in &["json", config::CONFIG_EXTENSION] {
            prune_file(&dir.join(format!("{stem}.{ext}")));
        }
    }

    Ok(())
}

/// Removes a single history file, logging a warning when it cannot be removed.
fn prune_file(path: &Path) {
    if let Err(e) = std::fs::remove_file(path) {
        eprintln!("Failed to prune history file {}: {e}", path.display());
    }
}

fn read_all_entries(dir: &Path) -> Result<Vec<HistoryEntry>> {
    std::fs::read_dir(dir)
        .context("Failed to read history dir")?
        .filter_map(core::result::Result::ok)
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
        .map(|entry| {
            let contents = std::fs::read_to_string(entry.path())
                .with_context(|| format!("Failed to read {}", entry.path().display()))?;
            serde_json::from_str(&contents)
                .with_context(|| format!("Failed to parse {}", entry.path().display()))
        })
        .collect()
}

fn read_all_stems(dir: &Path) -> Result<Vec<String>> {
    let stems: Vec<String> = std::fs::read_dir(dir)
        .context("Failed to read history dir")?
        .filter_map(core::result::Result::ok)
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
        .filter_map(|entry| {
            entry
                .path()
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_owned)
        })
        .collect();
    Ok(stems)
}

fn stem(timestamp: i64, update_id: &str) -> String {
    format!("{timestamp:020}-{update_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stem_is_sortable() {
        // ARRANGE
        let first = stem(1000, "update-1000");
        let second = stem(2000, "update-2000");

        // ACT & ASSERT
        assert!(first < second);
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
        let ts = 1_i64;
        let id = "abc";

        // ACT
        let name = stem(ts, id);

        // ASSERT
        assert_eq!(name.get(..20).unwrap_or_default(), "00000000000000000001");
        assert!(name.ends_with("-abc"));
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
            update_id: "update-1700000000".to_owned(),
            author: "alice".to_owned(),
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
            update_id: "update-42".to_owned(),
            author: "bob".to_owned(),
            change_kind: ChangeKind::Install,
        };
        let json = serde_json::to_string(&entry).expect("serialize");
        std::fs::write(dir.path().join("entry.json"), &json).unwrap();

        // ACT
        let entries = read_all_entries(dir.path()).expect("read_all_entries");

        // ASSERT
        assert_eq!(entries.len(), 1);
        let entry = entries.first().unwrap();
        assert_eq!(entry.update_id, "update-42");
        assert_eq!(entry.author, "bob");
        assert_eq!(entry.change_kind, ChangeKind::Install);
    }

    #[test]
    fn prune_removes_oldest_entries_beyond_max() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        for i in 0..5_u64 {
            let name = stem(i64::try_from(i).unwrap_or(0), &format!("update-{i}"));
            std::fs::write(dir.path().join(format!("{name}.json")), "{}").unwrap();
            std::fs::write(dir.path().join(format!("{name}.toml")), "").unwrap();
        }

        // ACT
        prune(dir.path(), 3).expect("prune");

        // ASSERT
        let mut remaining = read_all_stems(dir.path()).expect("read_all_stems");
        remaining.sort();
        assert_eq!(remaining.len(), 3);
        for stem_str in &remaining {
            let ts_part: u64 = stem_str
                .get(..20)
                .unwrap_or_default()
                .trim_start_matches('0')
                .parse()
                .unwrap_or(0);
            assert!(ts_part >= 2, "expected only ts>=2, got {ts_part}");
        }
    }

    #[test]
    fn prune_does_nothing_when_below_max() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        for i in 0..3_u64 {
            let name = stem(i64::try_from(i).unwrap_or(0), &format!("update-{i}"));
            std::fs::write(dir.path().join(format!("{name}.json")), "{}").unwrap();
        }

        // ACT
        prune(dir.path(), 10).expect("prune");

        // ASSERT
        let stems = read_all_stems(dir.path()).expect("read_all_stems");
        assert_eq!(stems.len(), 3);
    }
}
