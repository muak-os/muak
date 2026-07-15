//! Public artifact build API.

use std::io::Write;
use std::path::PathBuf;

use koci::pull::cache;
use sbolt::keys::SigningPair;
use serde::{Deserialize, Serialize};

use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::profile::Profile;
use crate::request::Request;
use crate::resolve::{self, Config};
use crate::source::{self, installer, overlay::Overlay};

pub(crate) mod archive;
pub(crate) mod artifacts;
pub(crate) mod media;
pub(crate) mod uki;

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

/// Artifact build metadata.
pub struct Metadata {
    /// PE section descriptors for the built UKI.
    pub sections: Vec<SectionInfo>,
    /// Overlay source information for deferred overlay pulling.
    pub overlay: Option<Overlay>,
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
    if request.artifacts.is_empty() {
        return Err(WizardError::BuildError(
            "at least one artifact must be requested".to_owned(),
        ));
    }

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

    let meta = installer::metadata(resolved.installer(), &resolved.arch(), None).await?;
    let needs_post = request
        .artifacts
        .iter()
        .any(|art| matches!(art, Artifact::Uki | Artifact::Iso | Artifact::Raw));
    let needs_tail = needs_post || initramfs.is_some();

    let extensions = if needs_tail {
        Some(source::extension::pull(&resolved).await?)
    } else {
        None
    };

    let (tail_parts, tail_size) = if needs_tail {
        let parts =
            archive::prepare_tail_parts(extensions.as_deref().unwrap_or(&[]), &profile_bytes)?;
        let size = archive::tail_exact_size(&parts);

        (Some(parts), size)
    } else {
        (None, 0)
    };

    let sections = if needs_post || kernel.is_some() || cmdline.is_some() || initramfs.is_some() {
        let error_msg = if needs_post {
            "tail parts required for post processing"
        } else {
            "tail parts required for initramfs"
        };
        let parts = tail_parts
            .as_ref()
            .ok_or_else(|| WizardError::BuildError(error_msg.to_owned()))?;

        let post_config = artifacts::BuildPostConfig {
            resolved: &resolved,
            installer_meta: &meta,
            tail_parts: parts,
            tail_size,
            signing_key,
        };
        artifacts::build(&post_config, uki, iso, raw, kernel, cmdline, initramfs).await?
    } else {
        Vec::default()
    };

    let overlay = resolved.overlay().cloned();

    Ok(Metadata {
        sections: sections
            .into_iter()
            .map(|section| SectionInfo {
                name: section.name.to_owned(),
                file_offset: section.file_offset,
                size: section.size,
                hash: section.checksum,
            })
            .collect(),
        overlay,
    })
}

/// Set the OCI blob cache directory for all image pulls performed by koci.
pub fn set_cache_dir<P: Into<PathBuf>>(path: P) {
    cache::Store::set_dir(path.into());
}
