//! Artifact rendering functions for staged installer inputs.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use esp::Arch as EspArch;
use esp::EspSpecBuilder;
use koci::arch::Arch;
use miso::error::MisoError;
use tokio::fs;
use tokio::task::spawn_blocking;

use crate::error::{ImagerError, Result};
use crate::request::Artifact;
use crate::source::model::ResolvedBuildProfile;
use crate::stage::InstallerAssets;

/// Builds the merged initramfs artifact from base image and extra files.
///
/// # Errors
///
/// Returns an error when ramune fails or output cannot be written.
pub async fn build_merged_initramfs(
    installer_assets: &InstallerAssets,
    extra_files: &[ramune::ExtraFile<'_>],
    output_dir: &Path,
) -> Result<PathBuf> {
    let output_path = output_dir.join(Artifact::Initramfs.filename());
    let config = ramune::ExtendConfig {
        base: &installer_assets.initramfs,
        extra_files,
        compression_level: ramune::DEFAULT_ZSTD_COMPRESSION_LEVEL,
    };

    ramune::extend(&config, &output_path)
        .await
        .map_err(|e| ImagerError::BuildError(format!("build initramfs: {e}")))?;

    Ok(output_path)
}

/// Builds the boot/install UKI and writes it to the output directory.
///
/// # Errors
///
/// Returns an error when reading assets, running yuki, or writing output fails.
pub async fn build_uki(
    installer_assets: &InstallerAssets,
    initramfs_path: &Path,
    output_dir: &Path,
) -> Result<PathBuf> {
    let output_path = output_dir.join(Artifact::Uki.filename());
    let stub = fs::read(&installer_assets.stub)
        .await
        .map_err(|e| ImagerError::BuildError(format!("read stub: {e}")))?;
    let kernel = fs::read(&installer_assets.kernel)
        .await
        .map_err(|e| ImagerError::BuildError(format!("read kernel: {e}")))?;
    let initramfs = fs::read(initramfs_path)
        .await
        .map_err(|e| ImagerError::BuildError(format!("read initramfs: {e}")))?;
    let cmdline = fs::read(&installer_assets.cmdline)
        .await
        .map_err(|e| ImagerError::BuildError(format!("read cmdline: {e}")))?;

    let uki_bytes = spawn_blocking(move || {
        yuki::build(&yuki::BuildInput {
            stub: &stub,
            kernel: &kernel,
            initramfs: &initramfs,
            cmdline: &cmdline,
            dtb: None,
            luks_key: None,
        })
        .map_err(|e| ImagerError::BuildError(format!("build UKI: {e}")))
    })
    .await
    .map_err(|e| ImagerError::BuildError(format!("join UKI build task: {e}")))?
    .map_err(|e| ImagerError::BuildError(format!("build UKI: {e}")))?;

    fs::write(&output_path, uki_bytes)
        .await
        .map_err(|e| ImagerError::BuildError(format!("write UKI: {e}")))?;

    Ok(output_path)
}

/// Builds the bootable ISO artifact from a prebuilt UKI.
///
/// # Errors
///
/// Returns an error when reading the UKI, running miso, or writing output fails.
pub async fn build_iso(
    resolved_profile: &ResolvedBuildProfile,
    output_dir: &Path,
    uki_path: &Path,
) -> Result<PathBuf> {
    let uki_bytes = fs::read(uki_path)
        .await
        .map_err(|e| ImagerError::BuildError(format!("read UKI: {e}")))?;
    let arch = esp_arch(resolved_profile.arch());
    let spec = EspSpecBuilder::default()
        .with_uki(arch, uki_bytes)
        .map_err(|e| ImagerError::BuildError(format!("add UKI to ISO ESP spec: {e}")))?
        .build()
        .map_err(|e| ImagerError::BuildError(format!("build ISO ESP spec: {e}")))?;
    let iso_bytes = spawn_blocking(move || {
        let mut out = Cursor::new(Vec::new());
        miso::build_iso(&spec, &mut out)?;
        Ok::<Vec<u8>, MisoError>(out.into_inner())
    })
    .await
    .map_err(|e| ImagerError::BuildError(format!("join ISO build task: {e}")))?
    .map_err(|e| ImagerError::BuildError(format!("build bootable ISO: {e}")))?;

    let iso_path = output_dir.join(Artifact::Iso.filename());
    fs::write(&iso_path, &iso_bytes)
        .await
        .map_err(|e| ImagerError::BuildError(format!("write ISO: {e}")))?;

    Ok(iso_path)
}

/// Builds the raw disk IMG artifact, layering any overlay boot assets.
///
/// # Errors
///
/// Returns an error when reading the UKI, running miso, or writing output fails.
pub async fn build_raw(
    resolved_profile: &ResolvedBuildProfile,
    overlay_assets: &[esp::EspFile],
    output_dir: &Path,
    uki_path: &Path,
) -> Result<PathBuf> {
    let uki_bytes = fs::read(uki_path)
        .await
        .map_err(|e| ImagerError::BuildError(format!("read UKI: {e}")))?;
    let arch = esp_arch(resolved_profile.arch());
    let spec = EspSpecBuilder::default()
        .with_uki(arch, uki_bytes)
        .map_err(|e| ImagerError::BuildError(format!("add UKI to raw ESP spec: {e}")))?
        .add_files(overlay_assets.to_vec())
        .map_err(|e| ImagerError::BuildError(format!("add overlay assets to raw ESP spec: {e}")))?
        .build()
        .map_err(|e| ImagerError::BuildError(format!("build raw ESP spec: {e}")))?;
    let raw_bytes = spawn_blocking(move || {
        let mut out = Cursor::new(Vec::new());
        miso::build_raw(&spec, &mut out, Some(6))?;
        Ok::<Vec<u8>, MisoError>(out.into_inner())
    })
    .await
    .map_err(|e| ImagerError::BuildError(format!("join IMG build task: {e}")))?
    .map_err(|e| ImagerError::BuildError(format!("build raw disk image: {e}")))?;

    let raw_path = output_dir.join(Artifact::Raw.filename());
    fs::write(&raw_path, &raw_bytes)
        .await
        .map_err(|e| ImagerError::BuildError(format!("write IMG: {e}")))?;

    Ok(raw_path)
}

fn esp_arch(arch: Arch) -> EspArch {
    match arch {
        Arch::Amd64 => EspArch::X86_64,
        Arch::Arm64 => EspArch::Aarch64,
    }
}
