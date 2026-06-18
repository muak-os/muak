//! Build pipeline orchestration.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sbolt::keys::SigningPair;
use sbolt::signature;
use tokio::fs;
use yuki::section::Section;

use super::archive;
use super::media;
use super::stage::{self, InstallerAssets};
use super::uki;
use crate::artifact::Artifact;
use crate::error::{ImagerError, Result};
use crate::resolve::ResolvedProfile;

/// Prebuilt intermediate artifacts shared across rendering paths.
pub(crate) struct Prepared {
    /// Loaded installer assets.
    pub assets: InstallerAssets,
    /// Final compressed append archive to concatenate after the base initramfs.
    pub initramfs_tail: Vec<u8>,
    /// Built UKI binary.
    pub uki_bytes: Vec<u8>,
    /// UKI PE sections for TPM sealing.
    pub sections: Vec<Section>,
}

/// Pulls the installer, builds the initramfs tail, and builds a UKI.
///
/// # Errors
///
/// Returns an error when pulling, staging, or building fails.
pub(crate) async fn prepare(
    resolved_profile: &ResolvedProfile,
    profile_bytes: &[u8],
) -> Result<Prepared> {
    let installer = stage::pull_installer(resolved_profile, None).await?;
    let assets = stage::load_installer_assets(&installer)?;
    let initramfs_tail = archive::build_initramfs_tail(resolved_profile, profile_bytes).await?;
    let (uki_bytes, sections) = uki::uki(&assets, &initramfs_tail).await?;

    Ok(Prepared {
        assets,
        initramfs_tail,
        uki_bytes,
        sections,
    })
}

/// Builds the requested artifacts sharing a single resolution.
///
/// # Errors
///
/// Returns an error when pulling, staging, building, or signing fails.
pub async fn artifacts(
    resolved_profile: &ResolvedProfile,
    requested: &[Artifact],
    signing_key: Option<&SigningPair<'_>>,
    profile_bytes: &[u8],
    output_dir: &Path,
) -> Result<HashMap<Artifact, PathBuf>> {
    let prepared = prepare(resolved_profile, profile_bytes).await?;

    let uki_bytes = if let Some(key) = signing_key {
        let capacity = prepared.uki_bytes.len().saturating_add(8192);
        let mut signed = Vec::with_capacity(capacity);
        signature::sign(
            &mut prepared.uki_bytes.as_slice(),
            key.signer,
            key.certificate,
            &mut signed,
        )
        .map_err(|e| ImagerError::BuildError(format!("sign UKI: {e}")))?;
        signed
    } else {
        prepared.uki_bytes
    };

    let mut results = HashMap::new();

    if requested.contains(&Artifact::Kernel) {
        let kernel_path = output_dir.join(Artifact::Kernel.filename());
        fs::write(
            &kernel_path,
            stage::read_file(&prepared.assets.kernel, "kernel")?,
        )
        .await
        .map_err(|e| ImagerError::BuildError(format!("write kernel: {e}")))?;
        results.insert(Artifact::Kernel, kernel_path);
    }
    if requested.contains(&Artifact::Cmdline) {
        let cmdline_path = output_dir.join(Artifact::Cmdline.filename());
        fs::write(
            &cmdline_path,
            stage::read_file(&prepared.assets.cmdline, "cmdline")?,
        )
        .await
        .map_err(|e| ImagerError::BuildError(format!("write cmdline: {e}")))?;
        results.insert(Artifact::Cmdline, cmdline_path);
    }
    if requested.contains(&Artifact::Initramfs) {
        let initramfs_path = output_dir.join(Artifact::Initramfs.filename());
        uki::write_initramfs(&prepared.assets, &prepared.initramfs_tail, &initramfs_path).await?;
        results.insert(Artifact::Initramfs, initramfs_path);
    }
    if requested.contains(&Artifact::Uki) {
        let uki_path = output_dir.join(Artifact::Uki.filename());
        fs::write(&uki_path, &uki_bytes)
            .await
            .map_err(|e| ImagerError::BuildError(format!("write UKI: {e}")))?;
        results.insert(Artifact::Uki, uki_path);
    }
    if requested.contains(&Artifact::Iso) {
        results.insert(
            Artifact::Iso,
            media::iso(resolved_profile, output_dir, &uki_bytes).await?,
        );
    }

    let needs_overlay = requested.contains(&Artifact::Raw) || requested.contains(&Artifact::Esp);
    let overlay_files = if needs_overlay {
        pull_overlay_if_present(resolved_profile).await?
    } else {
        vec![]
    };

    if requested.contains(&Artifact::Raw) {
        results.insert(
            Artifact::Raw,
            media::raw(resolved_profile, &overlay_files, output_dir, &uki_bytes).await?,
        );
    }
    if requested.contains(&Artifact::Esp) {
        let esp_dir = output_dir.join(Artifact::Esp.filename());
        media::write_esp_files(&overlay_files, &esp_dir).await?;
        results.insert(Artifact::Esp, esp_dir);
    }

    Ok(results)
}

pub(crate) async fn pull_overlay_if_present(
    resolved_profile: &ResolvedProfile,
) -> Result<Vec<esp::EspFile>> {
    if let Some(overlay) = resolved_profile.overlay() {
        stage::pull_overlay(overlay, &resolved_profile.arch(), None)
            .await
            .map_err(|e| ImagerError::BuildError(format!("pull overlay: {e}")))
    } else {
        Ok(vec![])
    }
}
