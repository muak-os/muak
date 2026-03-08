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
        dir.join(format!("{}.{}", stem, sysconfig::CONFIG_EXTENSION)),
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
        return std::fs::read_to_string(sysconfig::CONFIG_PATH)
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
        for ext in &["json", sysconfig::CONFIG_EXTENSION] {
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
    fn test_stem_is_sortable() {
        // ARRANGE
        let a = stem(1000, "update-1000");
        let b = stem(2000, "update-2000");

        // ACT & ASSERT
        assert!(a < b);
    }

    #[test]
    fn test_change_kind_display() {
        // ACT & ASSERT
        assert_eq!(ChangeKind::Install.to_string(), "install");
        assert_eq!(ChangeKind::Update.to_string(), "update");
        assert_eq!(ChangeKind::Rollback.to_string(), "rollback");
    }
}
