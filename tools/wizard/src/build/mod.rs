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
    let requested = &request.artifacts;

    let ArtifactWriters {
        uki,
        kernel,
        cmdline,
        initramfs,
        iso,
        raw,
    } = writers;

    let prepared = if requested.contains(&Artifact::Iso) || requested.contains(&Artifact::Raw) {
        // TODO: Buffer UKI in memory for EspSpec (ISO/Raw) — deferred optimization.
        let mut uki_buf = Vec::new();
        let prepared =
            prepare::prepare(&resolved, &profile_bytes, signing_key, &mut uki_buf).await?;
        if let Some(w) = uki {
            w.write_all(&uki_buf)
                .map_err(|e| WizardError::BuildError(format!("write UKI: {e}")))?;
        }
        if let Some(w) = iso {
            media::iso_to_writer(&resolved, &uki_buf, w).await?;
        }
        if let Some(w) = raw {
            let overlay = stage::pull_overlay_if_present(&resolved).await?;
            media::raw_to_writer(&resolved, &overlay, &uki_buf, w).await?;
        }
        prepared
    } else if let Some(w) = uki {
        prepare::prepare(&resolved, &profile_bytes, signing_key, w).await?
    } else {
        let installer = stage::pull_installer(&resolved, None)
            .await
            .map_err(|e| WizardError::BuildError(format!("pull installer: {e}")))?;
        let assets = stage::load_installer_assets(&installer)?;
        prepare::Prepared {
            assets,
            cached_extensions: Vec::new(),
            sections: Vec::new(),
            section_hashes: Vec::new(),
        }
    };

    if let Some(w) = kernel {
        let data = stage::read_file(&prepared.assets.kernel, "kernel")?;
        w.write_all(&data)
            .map_err(|e| WizardError::BuildError(format!("write kernel: {e}")))?;
    }
    if let Some(w) = cmdline {
        let data = stage::read_file(&prepared.assets.cmdline, "cmdline")?;
        w.write_all(&data)
            .map_err(|e| WizardError::BuildError(format!("write cmdline: {e}")))?;
    }
    if let Some(w) = initramfs {
        if prepared.cached_extensions.is_empty() {
            let mut base_reader = prepared
                .assets
                .initramfs
                .open()
                .map_err(|e| WizardError::BuildError(format!("open initramfs: {e}")))?;
            std::io::copy(&mut base_reader, w)
                .map_err(|e| WizardError::BuildError(format!("write initramfs base: {e}")))?;
            let tail = archive::build_initramfs_tail(&resolved, &profile_bytes).await?;
            w.write_all(&tail)
                .map_err(|e| WizardError::BuildError(format!("write initramfs tail: {e}")))?;
        } else {
            archive::write_combined_initramfs(
                &prepared.assets,
                &profile_bytes,
                &prepared.cached_extensions,
                w,
            )
            .await?;
        }
    }

    let overlay_files = stage::pull_overlay_if_present(&resolved).await?;

    Ok(Metadata {
        sections: prepared
            .sections
            .into_iter()
            .zip(prepared.section_hashes)
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
