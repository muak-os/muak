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
use crate::arch;
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
    let overlay = if iso.is_some() || raw.is_some() {
        match resolved.overlay() {
            Some(info) => source::pull_overlay(info).await?,
            None => Vec::new(),
        }
    } else {
        Vec::new()
    };

    let (mut uki_reader, uki_size, sections_handle) =
        uki::build(assets, tail_parts, tail_size, signing_key)?;

    if let Some(w) = iso {
        media::write_iso(
            arch::esp(resolved.arch()),
            &mut uki_reader,
            uki_size,
            overlay,
            w,
        )?;
    } else if let Some(w) = raw {
        media::write_raw(
            arch::esp(resolved.arch()),
            &mut uki_reader,
            uki_size,
            overlay,
            w,
        )?;
    } else if let Some(w) = uki {
        std::io::copy(&mut uki_reader, w)
            .map_err(|e| WizardError::BuildError(format!("write UKI: {e}")))?;
    } else {
        return Ok(Vec::default());
    }

    sections_handle
        .await
        .map_err(|e| WizardError::BuildError(format!("join UKI build task: {e}")))?
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
