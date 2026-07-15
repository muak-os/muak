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

/// An artifact type paired with its output writer.
pub enum ArtifactTarget<'a, W: Write> {
    /// Linux kernel image.
    Kernel(&'a mut W),
    /// Initial RAM filesystem image.
    Initramfs(&'a mut W),
    /// Kernel command-line file.
    Cmdline(&'a mut W),
    /// Unified kernel image (UKI) EFI binary.
    Uki(&'a mut W),
    /// ISO 9660 bootable image.
    Iso(&'a mut W),
    /// Raw disk image (compressed via zstd).
    Raw(&'a mut W),
}

impl<W: Write> ArtifactTarget<'_, W> {
    fn kind(&self) -> Artifact {
        match *self {
            Self::Kernel(_) => Artifact::Kernel,
            Self::Initramfs(_) => Artifact::Initramfs,
            Self::Cmdline(_) => Artifact::Cmdline,
            Self::Uki(_) => Artifact::Uki,
            Self::Iso(_) => Artifact::Iso,
            Self::Raw(_) => Artifact::Raw,
        }
    }
}

/// Builds the requested artifacts sharing a single resolution.
///
/// # Errors
///
/// Returns an error when resolution, pulling, building, or signing fails.
pub async fn artifacts<'a, W: Write + 'a>(
    request: &Request,
    profile: &Profile,
    config: &Config,
    signing_key: Option<&SigningPair<'_>>,
    targets: impl IntoIterator<Item = ArtifactTarget<'a, W>>,
) -> Result<Metadata> {
    let targets: Vec<ArtifactTarget<'a, W>> = targets.into_iter().collect();

    if targets.is_empty() {
        return Err(WizardError::BuildError(
            "at least one artifact must be requested".to_owned(),
        ));
    }

    for target in &targets {
        if !request.artifacts.contains(&target.kind()) {
            return Err(WizardError::BuildError(format!(
                "target artifact {:?} was not requested",
                target.kind()
            )));
        }
    }

    let mut uki: Option<&mut W> = None;
    let mut kernel: Option<&mut W> = None;
    let mut cmdline: Option<&mut W> = None;
    let mut initramfs: Option<&mut W> = None;
    let mut iso: Option<&mut W> = None;
    let mut raw: Option<&mut W> = None;

    for target in targets {
        match target {
            ArtifactTarget::Kernel(w) => kernel = Some(w),
            ArtifactTarget::Initramfs(w) => initramfs = Some(w),
            ArtifactTarget::Cmdline(w) => cmdline = Some(w),
            ArtifactTarget::Uki(w) => uki = Some(w),
            ArtifactTarget::Iso(w) => iso = Some(w),
            ArtifactTarget::Raw(w) => raw = Some(w),
        }
    }

    let resolved = resolve::profile(request, profile, &config.sources)?;

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
        let parts = archive::prepare_tail_parts(
            extensions.as_deref().unwrap_or(&[]),
            &profile.canonical_bytes()?,
        )?;
        let size = archive::tail_exact_size(&parts);

        (Some(parts), size)
    } else {
        (None, 0)
    };

    let post_config = artifacts::BuildPostConfig {
        resolved: &resolved,
        installer_meta: &meta,
        tail_parts: tail_parts.as_ref(),
        tail_size,
        signing_key,
    };
    let sections = artifacts::build(&post_config, uki, iso, raw, kernel, cmdline, initramfs).await?;

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
