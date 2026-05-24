//! Mounted-directory population helpers for ESP contents.

use std::path::Path;

use crate::error::Result;
use crate::{EspSpec, path};

/// Populates a mounted ESP directory from an `EspSpec`.
///
/// # Errors
///
/// Returns an error when the spec contains invalid paths or the destination cannot be
/// created or written.
pub fn populate(spec: &EspSpec, esp_root: &Path) -> Result<()> {
    path::validate_spec(spec)?;

    for file in &spec.files {
        let rel_path = path::validate_relative_path(&file.path)?;
        let dest = esp_root.join(rel_path);
        let mut parent = dest.clone();
        parent.pop();
        std::fs::create_dir_all(&parent)?;
        std::fs::write(dest, &file.data)?;
    }

    Ok(())
}
