//! Journal of config changes.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

/// Directory holding the journal entries.
pub(crate) const JOURNAL_DIR: &str = "/run/state/journal";

/// Maximum number of journal entries to retain.
const MAX_ENTRIES: usize = 1000;

/// Schema version of the journal entry document.
pub const API_VERSION: &str = "muak.dev/journal/v1-beta";

/// The kind of operation that produced a journal entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
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

/// One journal entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// Schema version of this document.
    api_version: String,
    /// Unix timestamp of the change.
    timestamp: i64,
    /// Update ID this entry belongs to.
    update_id: String,
    /// Client fingerprint or `system`.
    author: String,
    /// The kind of operation.
    kind: ChangeKind,
    /// The image that failed to boot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failed_image: Option<String>,
    /// Why the update was rolled back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

impl Entry {
    /// Creates a journal entry for the given change.
    #[must_use]
    pub fn new(update_id: &str, author: &str, kind: ChangeKind) -> Self {
        Self {
            api_version: API_VERSION.to_owned(),
            timestamp: 0,
            update_id: update_id.to_owned(),
            author: author.to_owned(),
            kind,
            failed_image: None,
            reason: None,
        }
    }

    /// Attaches rollback outcome fields to the entry.
    #[must_use]
    pub fn rolled_back(mut self, failed_image: &str, reason: &str) -> Self {
        self.failed_image = Some(failed_image.to_owned());
        self.reason = Some(reason.to_owned());

        self
    }

    /// Deserializes and validates a journal entry from TOML.
    ///
    /// # Errors
    ///
    /// Returns an error when parsing fails or the entry uses an unknown
    /// schema version.
    pub fn from_toml(bytes: &str) -> Result<Self> {
        let entry: Self = toml::from_str(bytes).context("Failed to parse journal entry")?;

        if entry.api_version != API_VERSION {
            bail!(
                "unsupported journal api_version '{}' (supported: {API_VERSION})",
                entry.api_version
            );
        }

        Ok(entry)
    }

    /// Returns the Unix timestamp of the change.
    #[must_use]
    pub const fn timestamp(&self) -> i64 {
        self.timestamp
    }

    /// Returns the update ID this entry belongs to.
    #[must_use]
    pub fn update_id(&self) -> &str {
        &self.update_id
    }

    /// Returns the author of the change.
    #[must_use]
    pub fn author(&self) -> &str {
        &self.author
    }

    /// Returns the kind of operation.
    #[must_use]
    pub const fn kind(&self) -> &ChangeKind {
        &self.kind
    }

    /// Returns the image that failed to boot, for rollback entries.
    #[must_use]
    pub fn failed_image(&self) -> Option<&str> {
        self.failed_image.as_deref()
    }

    /// Returns the rollback reason, for rollback entries.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    /// Serializes the journal entry to TOML.
    ///
    /// # Errors
    ///
    /// Returns an error when serialization fails.
    pub fn to_toml(&self) -> Result<String> {
        toml::to_string(self).context("Failed to serialize journal entry")
    }
}

/// Appends a journal entry and its config snapshot, then prunes old entries.
///
/// # Errors
///
/// Returns an error when the entry or its config snapshot cannot be written.
pub fn append(entry: &Entry, config: &str) -> Result<()> {
    let mut stamped = entry.clone();
    stamped.timestamp = now();

    let dir = Path::new(JOURNAL_DIR).join(stem(stamped.timestamp, &stamped.update_id));
    fs::create_dir_all(&dir).context("Failed to create journal entry dir")?;
    fs::write(dir.join("entry.toml"), stamped.to_toml()?)
        .context("Failed to write journal entry")?;
    fs::write(dir.join("config.toml"), config).context("Failed to write config snapshot")?;

    prune();

    Ok(())
}

/// Returns the most recent journal entries, newest-first, up to `limit`.
///
/// # Errors
///
/// Returns an error when an entry cannot be read.
pub fn list(limit: usize) -> Result<Vec<Entry>> {
    if !Path::new(JOURNAL_DIR).exists() {
        return Ok(Vec::new());
    }

    let mut dirs: Vec<PathBuf> = fs::read_dir(JOURNAL_DIR)
        .context("Failed to read journal dir")?
        .filter_map(core::result::Result::ok)
        .map(|entry| entry.path())
        .collect();
    dirs.sort_unstable();
    dirs.reverse();

    let mut entries = Vec::new();
    let found = dirs.into_iter().filter_map(|dir| read_entry(&dir).ok());
    for entry in found {
        entries.push(entry);
        if entries.len() >= limit {
            break;
        }
    }

    Ok(entries)
}

/// Returns the config snapshot recorded for `update_id`, or the live config when the ID is empty.
///
/// # Errors
///
/// Returns an error when the entry or its snapshot cannot be read.
pub fn snapshot(update_id: &str) -> Result<String> {
    if update_id.is_empty() {
        return fs::read_to_string(config::CONFIG_PATH).context("Failed to read current config");
    }

    let dir = find_entry_dir(update_id).context("No journal entry found for update ID")?;
    fs::read_to_string(dir.join("config.toml")).context("Failed to read journal config snapshot")
}

/// Returns the rollback reason recorded for `update_id`, if any.
pub fn find(update_id: &str) -> Option<Entry> {
    let dir = find_entry_dir(update_id)?;
    read_entry(&dir).ok()
}

fn read_entry(dir: &Path) -> Result<Entry> {
    let bytes =
        fs::read_to_string(dir.join("entry.toml")).context("Failed to read journal entry")?;

    Entry::from_toml(&bytes)
}

fn find_entry_dir(update_id: &str) -> Option<PathBuf> {
    fs::read_dir(JOURNAL_DIR)
        .ok()?
        .filter_map(core::result::Result::ok)
        .map(|entry| entry.path())
        .find(|dir| {
            dir.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(&format!("-{update_id}")))
        })
}

fn prune() {
    let mut dirs: Vec<PathBuf> = match fs::read_dir(JOURNAL_DIR) {
        Ok(rd) => rd
            .filter_map(core::result::Result::ok)
            .map(|entry| entry.path())
            .collect(),
        Err(_) => return,
    };

    if dirs.len() <= MAX_ENTRIES {
        return;
    }

    dirs.sort_unstable();
    let to_delete = dirs.len().saturating_sub(MAX_ENTRIES);
    for dir in dirs.iter().take(to_delete) {
        if let Err(e) = fs::remove_dir_all(dir) {
            eprintln!("Failed to prune journal entry {}: {e}", dir.display());
        }
    }
}

fn stem(timestamp: i64, update_id: &str) -> String {
    format!("{timestamp:020}-{update_id}")
}

fn now() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_journal() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let journal = dir.path().join("journal");
        (dir, journal)
    }

    fn write_entry(dir: &Path, entry: &Entry, config: &str) {
        std::fs::create_dir_all(dir).expect("create entry dir");
        std::fs::write(
            dir.join("entry.toml"),
            entry.to_toml().expect("serialize entry"),
        )
        .expect("write entry");
        std::fs::write(dir.join("config.toml"), config).expect("write config");
    }

    fn snapshot_from(dir: &Path, update_id: &str) -> Result<String> {
        let entry_dir = dir
            .read_dir()?
            .filter_map(core::result::Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(&format!("-{update_id}")))
            })
            .context("No journal entry found")?;
        fs::read_to_string(entry_dir.join("config.toml")).context("Failed to read snapshot")
    }

    fn rollback_from(dir: &Path, update_id: &str) -> Option<String> {
        let entry_dir = dir
            .read_dir()
            .ok()?
            .filter_map(core::result::Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(&format!("-{update_id}")))
            })?;
        read_entry(&entry_dir).ok()?.reason().map(str::to_owned)
    }

    fn prune_from(dir: &Path, max: usize) {
        let mut dirs: Vec<PathBuf> = fs::read_dir(dir)
            .expect("read dir")
            .filter_map(core::result::Result::ok)
            .map(|entry| entry.path())
            .collect();
        if dirs.len() <= max {
            return;
        }
        dirs.sort_unstable();
        let to_delete = dirs.len().saturating_sub(max);
        for path in dirs.iter().take(to_delete) {
            fs::remove_dir_all(path).expect("prune");
        }
    }

    fn list_from(dir: &Path, limit: usize) -> Result<Vec<Entry>> {
        let mut dirs: Vec<PathBuf> = fs::read_dir(dir)?
            .filter_map(core::result::Result::ok)
            .map(|entry| entry.path())
            .collect();
        dirs.sort_unstable();
        dirs.reverse();

        let entries: Vec<Entry> = dirs
            .into_iter()
            .filter_map(|dir| read_entry(&dir).ok())
            .take(limit)
            .collect();

        Ok(entries)
    }

    #[test]
    fn stem_is_sortable() {
        // ARRANGE / ACT / ASSERT
        assert!(stem(1000, "a") < stem(2000, "b"));
        assert_eq!(stem(1, "install"), "00000000000000000001-install");
    }

    #[test]
    fn entry_toml_round_trip_preserves_fields() {
        // ARRANGE
        let mut entry = Entry::new("update-1734", "a1b2", ChangeKind::Update);
        entry.timestamp = 1_700_000_000;

        // ACT
        let serialized = entry.to_toml().expect("serialize");
        let parsed = Entry::from_toml(&serialized).expect("deserialize");

        // ASSERT
        assert_eq!(parsed, entry);
        assert!(serialized.contains("api_version = \"muak.dev/journal/v1-beta\""));
    }

    #[test]
    fn rollback_entry_round_trips_outcome_fields() {
        // ARRANGE / ACT
        let mut entry = Entry::new("update-1734", "system", ChangeKind::Rollback)
            .rolled_back("ghcr.io/muak-os/installer:v2", "health check failed");
        entry.timestamp = 1;
        let parsed = Entry::from_toml(&entry.to_toml().expect("serialize")).expect("deserialize");

        // ASSERT
        assert_eq!(parsed.kind(), &ChangeKind::Rollback);
        assert_eq!(parsed.failed_image(), Some("ghcr.io/muak-os/installer:v2"));
        assert_eq!(parsed.reason(), Some("health check failed"));
    }

    #[test]
    fn plain_entry_has_no_rollback_fields() {
        // ARRANGE / ACT
        let mut entry = Entry::new("install", "system", ChangeKind::Install);
        entry.timestamp = 1;
        let serialized = entry.to_toml().expect("serialize");

        // ASSERT
        assert!(!serialized.contains("failed_image"));
        assert!(!serialized.contains("reason"));
    }

    #[test]
    fn from_toml_rejects_unknown_api_version() {
        // ARRANGE
        let bytes = "api_version = \"muak.dev/journal/v999\"\ntimestamp = 1\nupdate_id = \"u\"\nauthor = \"a\"\nkind = \"update\"\n";

        // ACT / ASSERT
        let error = Entry::from_toml(bytes).unwrap_err();
        assert!(error.to_string().contains("api_version"), "{error}");
    }

    #[test]
    fn from_toml_rejects_missing_api_version() {
        // ARRANGE
        let bytes = "timestamp = 1\nupdate_id = \"u\"\nauthor = \"a\"\nkind = \"update\"\n";

        // ACT / ASSERT
        let error = Entry::from_toml(bytes).unwrap_err();
        assert!(error.to_string().contains("Failed to parse"), "{error}");
    }

    #[test]
    fn record_list_and_snapshot_round_trip() {
        // ARRANGE
        let (_guard, dir) = temp_journal();
        let first = stem(1000, "install");
        let second = stem(2000, "update-1734");
        write_entry(
            &dir.join(&first),
            &Entry::new("install", "system", ChangeKind::Install),
            "before = true",
        );
        write_entry(
            &dir.join(&second),
            &Entry::new("update-1734", "a1b2", ChangeKind::Update),
            "before = false",
        );

        // ACT
        let entries = list_from(&dir, 10).expect("list");
        let snapshot = snapshot_from(&dir, "update-1734").expect("snapshot");

        // ASSERT
        assert_eq!(entries.len(), 2, "both entries must be listed");
        assert_eq!(
            entries.first().expect("entry").update_id(),
            "update-1734",
            "newest first"
        );
        assert_eq!(snapshot, "before = false");
    }

    #[test]
    fn rollback_lookup_returns_reason() {
        // ARRANGE
        let (_guard, dir) = temp_journal();
        write_entry(
            &dir.join(stem(1000, "update-1734")),
            &Entry::new("update-1734", "system", ChangeKind::Rollback)
                .rolled_back("img:v2", "boom"),
            "old = true",
        );

        // ACT / ASSERT
        assert_eq!(rollback_from(&dir, "update-1734").as_deref(), Some("boom"));
        assert!(rollback_from(&dir, "update-9999").is_none());
    }

    #[test]
    fn prune_removes_oldest_entries_beyond_max() {
        // ARRANGE
        let (_guard, dir) = temp_journal();
        for ts in 0..5_i64 {
            let id = format!("update-{ts}");
            write_entry(
                &dir.join(stem(ts, &id)),
                &Entry::new(&id, "system", ChangeKind::Update),
                "config",
            );
        }

        // ACT
        prune_from(&dir, 3);

        // ASSERT
        let remaining: Vec<_> = fs::read_dir(&dir)
            .expect("read dir")
            .filter_map(core::result::Result::ok)
            .map(|entry| entry.path())
            .collect();
        assert_eq!(remaining.len(), 3, "oldest entries must be pruned");
    }
}
