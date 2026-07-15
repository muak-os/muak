//! Extension OCI image metadata extraction and file pulling.

use std::io::Read;

use koci::arch::Arch;
use koci::pull::{
    self,
    entries::{FileEntry, MetadataEntry},
};

use crate::error::{Result, WizardError};
use crate::resolve::BuildPlan;

/// A reference to an extension source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extension {
    name: String,
    source: String,
}

impl Extension {
    #[must_use]
    pub(crate) fn new(name: String, source: String) -> Self {
        Self { name, source }
    }

    /// Returns the canonical logical extension name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the versioned OCI reference for this extension.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// Extension metadata extracted from an OCI image.
pub struct Metadata {
    /// Extension name.
    pub name: String,
    /// Files in the extension: (path, size, mode).
    pub files: Vec<(String, u64, u32)>,
}

/// Extracts metadata from each resolved extension OCI image.
///
/// # Errors
///
/// Returns an error when any extension OCI metadata extraction fails.
pub async fn metadata(
    extensions: &[Extension],
    arch: &Arch,
    signature_public_key: Option<&str>,
) -> Result<Vec<Metadata>> {
    let mut result = Vec::with_capacity(extensions.len());
    for ext in extensions {
        let mut files = Vec::new();
        pull::metadata(
            ext.source(),
            arch,
            signature_public_key,
            |entry: MetadataEntry| {
                files.push((entry.path, entry.size, entry.mode));
                Ok(())
            },
        )
        .await
        .map_err(|e| {
            WizardError::BuildError(format!("extract extension {} metadata: {e}", ext.source()))
        })?;
        result.push(Metadata {
            name: ext.name().to_owned(),
            files,
        });
    }

    Ok(result)
}

/// Pulls extension metadata and buffers file data.
///
/// # Errors
///
/// Returns an error when any extension OCI metadata extraction or file pull fails.
pub(crate) async fn pull(
    resolved_profile: &BuildPlan,
) -> Result<Vec<(String, Metadata, Vec<Vec<u8>>)>> {
    let resolved_extensions = resolved_profile.extensions();
    if resolved_extensions.is_empty() {
        return Ok(vec![]);
    }

    let metadata_list = metadata(resolved_extensions, &resolved_profile.arch(), None).await?;

    let mut result = Vec::with_capacity(metadata_list.len());
    for (ext_ref, meta) in resolved_extensions.iter().zip(metadata_list) {
        let mut buffered_data = Vec::with_capacity(meta.files.len());

        pull::files(
            ext_ref.source(),
            &resolved_profile.arch(),
            None,
            |entry: FileEntry| {
                let capacity = usize::try_from(entry.size).unwrap_or(usize::MAX);
                let mut data = Vec::with_capacity(capacity);
                Read::read_to_end(entry.reader, &mut data)?;
                buffered_data.push(data);

                Ok(())
            },
        )
        .await
        .map_err(|e| {
            WizardError::BuildError(format!("pull extension {}: {e}", ext_ref.source()))
        })?;

        let name = meta.name.clone();
        result.push((name, meta, buffered_data));
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_extension_accessors() {
        // ARRANGE
        let ext = Extension::new("muak-os/qemu".into(), "ghcr.io/muak-os/qemu:v1.0.0".into());

        // ACT & ASSERT
        assert_eq!(ext.name(), "muak-os/qemu");
        assert_eq!(ext.source(), "ghcr.io/muak-os/qemu:v1.0.0");
    }
}
