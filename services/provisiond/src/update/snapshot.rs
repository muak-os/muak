//! Config snapshot life cycle: create, locate, read, and restore.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use config::{CONFIG_EXTENSION, CONFIG_PATH};

use crate::constants::UPDATE_DIR;
use crate::history::{self, ChangeKind};

/// Generates a unique update ID and saves a copy of the current config to `UPDATE_DIR`.
pub fn create(staging_dir: &Path) -> Result<String> {
    let update_id = generate_id();
    let contents =
        fs::read_to_string(CONFIG_PATH).context("Failed to read current config for snapshot")?;
    fs::write(
        staging_dir.join(format!("{}.{}", update_id, CONFIG_EXTENSION)),
        contents,
    )
    .context("Failed to write config snapshot")?;
    Ok(update_id)
}

/// Scans `UPDATE_DIR` for a pending snapshot and returns `(update_id, path)` if found.
pub fn find_pending() -> Result<Option<(String, PathBuf)>> {
    let dir = Path::new(UPDATE_DIR);
    if !dir.exists() {
        return Ok(None);
    }

    let entry = fs::read_dir(dir)
        .context("Failed to read update dir")?
        .filter_map(|e| e.ok())
        .find(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy();
            s.starts_with("update-") && s.ends_with(&format!(".{}", CONFIG_EXTENSION))
        });

    let Some(entry) = entry else {
        return Ok(None);
    };

    let path = entry.path();
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .context("Snapshot path has no file stem")?;

    Ok(Some((stem, path)))
}

/// Returns the path to the snapshot file for a given update ID.
pub fn path(update_id: &str) -> PathBuf {
    Path::new(UPDATE_DIR).join(format!("{}.{}", update_id, CONFIG_EXTENSION))
}

pub fn find(dir: &Path, update_id: &str) -> Result<PathBuf> {
    let path = std::fs::read_dir(dir)
        .context("Failed to read dir")?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.extension().and_then(|s| s.to_str()) == Some(CONFIG_EXTENSION)
                && p.file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.ends_with(update_id))
                    .unwrap_or(false)
        });

    path.with_context(|| format!("No snapshot found for update_id '{}'", update_id))
}

/// Reads `host.image` from a snapshot file.
pub fn read_image(snapshot_path: &Path) -> Result<String> {
    let contents = fs::read_to_string(snapshot_path).context("Failed to read config snapshot")?;
    let cfg: config::SystemConfig =
        config::parse_from_str(&contents).context("Failed to parse config snapshot")?;
    Ok(cfg.host.image)
}

/// Restores the system config from a snapshot file, overwriting the current, and records history.
pub fn restore(update_id: &str, snapshot_path: &Path) -> Result<()> {
    let contents = fs::read_to_string(snapshot_path).context("Failed to read config snapshot")?;
    fs::write(CONFIG_PATH, &contents).context("Failed to restore config from snapshot")?;

    if let Err(e) = history::record(update_id, "system", ChangeKind::Rollback, &contents) {
        eprintln!("Failed to record rollback history: {}", e);
    }

    Ok(())
}

fn generate_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("update-{}", timestamp)
}
