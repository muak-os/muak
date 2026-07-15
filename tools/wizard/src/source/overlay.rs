//! Overlay OCI image metadata extraction and file pulling.

use std::io::Read;

use esp::FileMeta;
use koci::arch::Arch;
use koci::error::KociError;
use koci::pull::{
    self,
    entries::{FileEntry, MetadataEntry},
};

use crate::error::{Result, WizardError};

/// An overlay source resolved from the profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlay {
    /// Overlay name inside the OCI image.
    pub name: String,
    /// Logical overlay image name.
    pub image: String,
    /// Versioned OCI reference for the overlay image.
    pub source: String,
    /// Target architecture of the overlay.
    pub arch: Arch,
}

impl Overlay {
    #[must_use]
    pub(crate) fn new(name: String, image: String, source: String, arch: Arch) -> Self {
        Self {
            name,
            image,
            source,
            arch,
        }
    }

    /// Returns the selected overlay name inside the OCI image.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the logical overlay image name.
    #[must_use]
    pub fn image(&self) -> &str {
        &self.image
    }

    /// Returns the versioned OCI reference for this overlay image.
    #[must_use]
    pub fn source_ref(&self) -> &str {
        &self.source
    }
}

/// Extracts overlay file metadata from the overlay OCI image.
///
/// # Errors
///
/// Returns an error when the OCI metadata extraction fails.
pub async fn metadata(overlay: &Overlay) -> Result<Vec<FileMeta<'static>>> {
    let mut files = Vec::new();
    let prefix = format!("{}/", overlay.name);

    pull::metadata(
        &overlay.source,
        &overlay.arch,
        None,
        |entry: MetadataEntry| {
            if let Some(rel) = entry.path.strip_prefix(&prefix)
                && !rel.is_empty()
            {
                files.push(FileMeta::new(rel.to_owned().leak(), entry.size));
            }
            Ok(())
        },
    )
    .await
    .map_err(|e| WizardError::BuildError(format!("extract overlay metadata: {e}")))?;

    files.sort_unstable_by(|left, right| left.path.cmp(right.path));

    Ok(files)
}

/// Pulls overlay files from the overlay OCI image, calling `on_entry` for each
/// matching file with its relative path, size, and readable stream.
///
/// # Errors
///
/// Returns an error when the OCI pull fails or the handler returns an error.
pub async fn pull<F>(overlay: &Overlay, mut on_entry: F) -> Result<()>
where
    F: FnMut(&str, u64, &mut dyn Read) -> Result<()>,
{
    let prefix = format!("{}/", overlay.name);

    pull::files(&overlay.source, &overlay.arch, None, |entry: FileEntry| {
        if let Some(rel) = entry.path.strip_prefix(&prefix)
            && !rel.is_empty()
            && let Err(e) = on_entry(rel, entry.size, entry.reader)
        {
            return Err(KociError::IoError(std::io::Error::other(e)));
        }

        Ok(())
    })
    .await
    .map_err(|e| WizardError::BuildError(format!("pull overlay files: {e}")))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_overlay_accessors() {
        // ARRANGE
        let ov = Overlay::new(
            "rpi_generic".into(),
            "muak-os/sbc-raspberrypi".into(),
            "ghcr.io/muak-os/sbc-raspberrypi:v1.0.0".into(),
            Arch::Amd64,
        );

        // ACT & ASSERT
        assert_eq!(ov.name(), "rpi_generic");
        assert_eq!(ov.image(), "muak-os/sbc-raspberrypi");
        assert_eq!(ov.source_ref(), "ghcr.io/muak-os/sbc-raspberrypi:v1.0.0");
    }
}
