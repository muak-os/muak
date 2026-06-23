//! Public artifact build API.

use std::io::Write;

use sbolt::keys::SigningPair;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::profile::Profile;
use crate::request::Request;
use crate::resolve::{self, Config};

pub(crate) mod archive;
pub(crate) mod media;
pub(crate) mod pipeline;
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

    let meta = pipeline::artifacts(
        &resolved,
        &request.artifacts,
        signing_key,
        &profile_bytes,
        writers,
    )
    .await?;

    Ok(Metadata {
        sections: meta
            .sections
            .into_iter()
            .zip(meta.section_hashes)
            .map(|(section, hash)| SectionInfo {
                name: section.name.to_owned(),
                file_offset: section.file_offset,
                size: section.size,
                hash,
            })
            .collect(),
        overlay_files: meta.overlay_files,
    })
}
