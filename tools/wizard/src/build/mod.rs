//! Public artifact build API.

use std::io::Write;

use sbolt::keys::SigningPair;
use serde::{Deserialize, Serialize};

use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::profile::Profile;
use crate::request::Request;
use crate::resolve::{self, Config};

pub(crate) mod archive;
pub(crate) mod artifacts;
pub(crate) mod media;
pub(crate) mod prepare;
pub(crate) mod stage;

/// PE section metadata needed for TPM PCR#11 prediction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionInfo {
    /// PE section name (e.g. ".linux", ".initrd", ".cmdline").
    pub name: String,
    /// File offset of the section data within the PE image.
    pub file_offset: usize,
    /// Size of the section data in bytes.
    pub size: usize,
    /// SHA-256 hash of the section data.
    pub hash: [u8; 32],
}

/// Artifact build metadata (PE sections, overlay files).
pub struct Metadata {
    /// PE section descriptors for the built UKI.
    pub sections: Vec<SectionInfo>,
    /// Overlay boot assets pulled from the resolved overlay image.
    pub overlay_files: Vec<esp::EspFile>,
}

/// Per-artifact output sinks passed to [`artifacts`].
pub struct ArtifactWriters<'a, W: Write> {
    /// Sink for the signed UKI `.efi` file.
    pub uki: Option<&'a mut W>,
    /// Sink for the extracted kernel image.
    pub kernel: Option<&'a mut W>,
    /// Sink for the kernel command line.
    pub cmdline: Option<&'a mut W>,
    /// Sink for the combined initramfs (base + tail).
    pub initramfs: Option<&'a mut W>,
    /// Sink for the bootable ISO image.
    pub iso: Option<&'a mut W>,
    /// Sink for the raw disk image.
    pub raw: Option<&'a mut W>,
}

/// Builds the requested artifacts sharing a single resolution.
///
/// # Errors
///
/// Returns an error when resolution, pulling, building, or signing fails.
pub async fn artifacts<W: Write>(
    request: &Request,
    profile: &Profile,
    config: &Config,
    signing_key: Option<&SigningPair<'_>>,
    writers: ArtifactWriters<'_, W>,
) -> Result<Metadata> {
    let resolved = resolve::profile(request, profile, &config.sources)?;
    let profile_bytes = profile.canonical_bytes()?;
    let ArtifactWriters {
        uki,
        kernel,
        cmdline,
        initramfs,
        iso,
        raw,
    } = writers;

    let assets = artifacts::pull_installer_assets(&resolved).await?;
    let extensions = artifacts::prepare_extensions(&resolved, &request.artifacts).await?;
    let needs_post = request
        .artifacts
        .iter()
        .any(|art| matches!(art, Artifact::Uki | Artifact::Iso | Artifact::Raw));
    let needs_tail = needs_post || initramfs.is_some();

    let (tail_parts, tail_size) = if needs_tail {
        let parts =
            archive::prepare_tail_parts(extensions.as_deref().unwrap_or(&[]), &profile_bytes)?;
        let size = archive::tail_exact_size(&parts);

        (Some(parts), size)
    } else {
        (None, 0)
    };

    let (sections, section_hashes) = if needs_post {
        let parts = tail_parts.as_ref().ok_or_else(|| {
            WizardError::BuildError("tail parts required for post processing".to_owned())
        })?;
        artifacts::build_post(
            &assets,
            &resolved,
            parts,
            tail_size,
            signing_key,
            uki,
            iso,
            raw,
        )
        .await?
    } else {
        Default::default()
    };

    artifacts::write_standalone(&assets, tail_parts.as_ref(), kernel, cmdline, initramfs)?;

    let overlay_files = stage::pull_overlay(&resolved).await?;

    Ok(Metadata {
        sections: sections
            .into_iter()
            .zip(section_hashes)
            .map(|(section, hash)| SectionInfo {
                name: section.name.to_owned(),
                file_offset: section.file_offset,
                size: section.size,
                hash,
            })
            .collect(),
        overlay_files,
    })
}
