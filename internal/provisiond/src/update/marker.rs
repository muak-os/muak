//! Validation marker for tracking update state across reboots.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::constants::UPDATE_DIR;

/// Marker file tracking update validation state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationMarker {
    pub update_id: String,
    pub target_image: String,
    pub old_image: String,
    pub timestamp: i64,
}

/// Creates a validation marker for the given target image.
pub fn create(target_image: &str) -> Result<ValidationMarker> {
    let old_image = sysconfig::system().image.clone();

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    Ok(ValidationMarker {
        update_id: format!("update-{}", timestamp),
        target_image: target_image.to_string(),
        old_image,
        timestamp,
    })
}

/// Saves the validation marker to the staging directory.
pub fn save(staging_dir: &Path, marker: &ValidationMarker) -> Result<()> {
    let marker_json = serde_json::to_string_pretty(marker)?;
    let marker_path = staging_dir.join("pending-validation.json");
    std::fs::write(marker_path, marker_json).context("Failed to write validation marker")
}

/// Loads the validation marker from disk if it exists.
pub fn load() -> Result<Option<ValidationMarker>> {
    let marker_path = Path::new(UPDATE_DIR).join("pending-validation.json");

    if !marker_path.exists() {
        return Ok(None);
    }

    let contents =
        std::fs::read_to_string(&marker_path).context("Failed to read pending-validation.json")?;

    let marker: ValidationMarker =
        serde_json::from_str(&contents).context("Failed to parse pending-validation.json")?;

    Ok(Some(marker))
}
