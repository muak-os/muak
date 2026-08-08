//! Extension OCI source references and file pulling.

use koci::arch::Arch;
use koci::error::KociError;
use koci::pull;
use koci::pull::entries::FileEntry;

use crate::error::{Result, WizardError};

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

/// Pulls extension files and assembles one opaque image payload per extension.
///
/// # Errors
///
/// Returns an error when any extension OCI file pull or payload assembly fails.
pub(crate) async fn pull(
    extensions: &[Extension],
    arch: &Arch,
) -> Result<Vec<mumi::payload::Payload>> {
    let mut payloads = Vec::with_capacity(extensions.len());

    for ext in extensions {
        let mut payload = mumi::payload::Payload::new(ext.name());
        pull::files(ext.source(), arch, None, |entry| {
            add_entry(&mut payload, entry)
        })
        .await
        .map_err(|e| WizardError::BuildError(format!("pull extension {}: {e}", ext.source())))?;
        payloads.push(payload);
    }

    Ok(payloads)
}

/// Streams one OCI entry into the payload, mapping it to an image file.
fn add_entry(
    payload: &mut mumi::payload::Payload,
    entry: FileEntry<'_>,
) -> koci::error::Result<()> {
    let path = entry.path.clone();
    let reader = entry.reader;
    let file = mumi::payload::FileEntry {
        path: format!("/{path}"),
        size: entry.size,
        mode: 0o100_000 | entry.mode,
    };
    payload.add_file(file, reader).map_err(|e| {
        KociError::IoError(std::io::Error::other(format!(
            "add extension file {path}: {e}"
        )))
    })
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
