//! Artifact build orchestration helpers.

use std::io::Write;

use koci::pulled::PulledImage;
use sbolt::keys::SigningPair;
use yuki::section::Section;

use super::archive;
use super::archive::TailParts;
use super::media;
use super::source;
use super::source::InstallerAssets;
use super::uki;
use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::resolve::BuildPlan;

pub(crate) async fn pull_installer_assets(resolved: &BuildPlan) -> Result<source::InstallerAssets> {
    let installer = source::pull_installer(resolved, None).await?;

    source::load_installer_assets(&installer)
}

pub(crate) async fn prepare_extensions(
    resolved: &BuildPlan,
    requested: &[Artifact],
) -> Result<Option<Vec<(String, PulledImage)>>> {
    if requested.iter().any(|art| {
        matches!(
            art,
            Artifact::Uki | Artifact::Iso | Artifact::Raw | Artifact::Initramfs
        )
    }) {
        Ok(Some(archive::pull_extensions(resolved).await?))
    } else {
        Ok(None)
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "internal function taking post-processing inputs"
)]
pub(crate) async fn build_post<W: Write>(
    assets: &InstallerAssets,
    resolved: &BuildPlan,
    tail_parts: &TailParts,
    tail_size: u64,
    signing_key: Option<&SigningPair<'_>>,
    uki: Option<&mut W>,
    iso: Option<&mut W>,
    raw: Option<&mut W>,
) -> Result<Vec<Section>> {
    if let Some(w) = iso {
        let (reader, uki_size, sections_handle) =
            uki::build(assets, tail_parts, tail_size, signing_key)?;
        media::iso_to_writer(resolved, reader, uki_size, w)?;
        sections_handle
            .await
            .map_err(|e| WizardError::BuildError(format!("join UKI build task: {e}")))?
    } else if let Some(w) = raw {
        let overlay = match resolved.overlay() {
            Some(info) => source::pull_overlay(info).await?,
            None => Vec::new(),
        };
        let (reader, uki_size, sections_handle) =
            uki::build(assets, tail_parts, tail_size, signing_key)?;
        media::raw_to_writer(resolved, overlay, reader, uki_size, w)?;
        sections_handle
            .await
            .map_err(|e| WizardError::BuildError(format!("join UKI build task: {e}")))?
    } else if let Some(w) = uki {
        let (mut reader, _uki_size, sections_handle) =
            uki::build(assets, tail_parts, tail_size, signing_key)?;
        std::io::copy(&mut reader, w)
            .map_err(|e| WizardError::BuildError(format!("write UKI: {e}")))?;
        sections_handle
            .await
            .map_err(|e| WizardError::BuildError(format!("join UKI build task: {e}")))?
    } else {
        Ok(Vec::default())
    }
}

pub(crate) fn write_standalone<W: Write>(
    assets: &InstallerAssets,
    tail_parts: Option<&TailParts>,
    kernel: Option<&mut W>,
    cmdline: Option<&mut W>,
    initramfs: Option<&mut W>,
) -> Result<()> {
    if let Some(w) = kernel {
        let data = source::read_file(&assets.kernel, "kernel")?;
        w.write_all(&data)
            .map_err(|e| WizardError::BuildError(format!("write kernel: {e}")))?;
    }
    if let Some(w) = cmdline {
        let data = source::read_file(&assets.cmdline, "cmdline")?;
        w.write_all(&data)
            .map_err(|e| WizardError::BuildError(format!("write cmdline: {e}")))?;
    }
    if let Some(w) = initramfs {
        let tail = tail_parts.ok_or_else(|| {
            WizardError::BuildError("initramfs requires extensions but none were pulled".to_owned())
        })?;
        archive::write_combined_initramfs(assets, tail, w)?;
    }

    Ok(())
}
