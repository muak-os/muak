//! Artifact build orchestration helpers.

use std::io::Write;

use koci::pulled::PulledImage;
use sbolt::keys::SigningPair;
use yuki::section::Section;

use super::archive;
use super::archive::TailParts;
use super::media;
use super::prepare;
use super::stage;
use super::stage::InstallerAssets;
use crate::artifact::Artifact;
use crate::error::{Result, WizardError};
use crate::resolve::ResolvedProfile;

pub(crate) async fn pull_installer_assets(
    resolved: &ResolvedProfile,
) -> Result<stage::InstallerAssets> {
    let installer = stage::pull_installer(resolved, None).await?;

    stage::load_installer_assets(&installer)
}

pub(crate) async fn prepare_extensions(
    resolved: &ResolvedProfile,
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
    resolved: &ResolvedProfile,
    tail_parts: &TailParts,
    tail_size: u64,
    signing_key: Option<&SigningPair<'_>>,
    uki: Option<&mut W>,
    iso: Option<&mut W>,
    raw: Option<&mut W>,
) -> Result<Vec<Section>> {
    let iso_or_raw = iso.is_some() || raw.is_some();
    if iso_or_raw {
        // TODO: Avoid buffering the full UKI for ISO/Raw — stream it instead.
        let mut uki_buf = Vec::new();
        let result =
            prepare::build_uki(assets, tail_parts, tail_size, signing_key, &mut uki_buf).await?;
        if let Some(w) = uki {
            w.write_all(&uki_buf)
                .map_err(|e| WizardError::BuildError(format!("write UKI: {e}")))?;
        }
        if let Some(w) = iso {
            media::iso_to_writer(resolved, &uki_buf, w).await?;
        }
        if let Some(w) = raw {
            let overlay = stage::pull_overlay(resolved).await?;
            media::raw_to_writer(resolved, &overlay, &uki_buf, w).await?;
        }

        Ok(result)
    } else if let Some(w) = uki {
        prepare::build_uki(assets, tail_parts, tail_size, signing_key, w).await
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
        let data = stage::read_file(&assets.kernel, "kernel")?;
        w.write_all(&data)
            .map_err(|e| WizardError::BuildError(format!("write kernel: {e}")))?;
    }
    if let Some(w) = cmdline {
        let data = stage::read_file(&assets.cmdline, "cmdline")?;
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
