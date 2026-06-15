//! Bootable media builders (ISO, raw disk image, ESP overlay).

use std::fs::File;
use std::path::{Path, PathBuf};

use esp::Arch as EspArch;
use esp::EspSpecBuilder;
use koci::arch::Arch;
use tokio::fs;
use tokio::task::spawn_blocking;

use crate::artifact::Artifact;
use crate::error::{ImagerError, Result};
use crate::resolve::ResolvedProfile;

/// Build a bootable ISO image from a UKI.
///
/// # Errors
///
/// Returns an error when creating the ISO or writing it fails.
pub async fn iso(
    resolved_profile: &ResolvedProfile,
    output_dir: &Path,
    uki_bytes: &[u8],
) -> Result<PathBuf> {
    let arch = esp_arch(resolved_profile.arch());
    let spec = EspSpecBuilder::default()
        .with_uki(arch, uki_bytes.to_vec())
        .map_err(|e| ImagerError::BuildError(format!("add UKI to ISO ESP spec: {e}")))?
        .build()
        .map_err(|e| ImagerError::BuildError(format!("build ISO ESP spec: {e}")))?;

    let iso_path = output_dir.join(Artifact::Iso.filename());
    let iso_path_clone = iso_path.clone();
    spawn_blocking(move || {
        let file = File::create(&iso_path_clone).map_err(std::io::Error::other)?;
        let mut writer = std::io::BufWriter::new(file);
        miso::build_iso(&spec, &mut writer).map_err(std::io::Error::other)
    })
    .await
    .map_err(|e| ImagerError::BuildError(format!("join ISO build task: {e}")))?
    .map_err(|e| ImagerError::BuildError(format!("build bootable ISO: {e}")))?;

    Ok(iso_path)
}

/// Build a raw disk image from a UKI and optional overlay assets.
///
/// # Errors
///
/// Returns an error when creating the raw image or writing it fails.
pub async fn raw(
    resolved_profile: &ResolvedProfile,
    overlay_assets: &[esp::EspFile],
    output_dir: &Path,
    uki_bytes: &[u8],
) -> Result<PathBuf> {
    let arch = esp_arch(resolved_profile.arch());
    let spec = EspSpecBuilder::default()
        .with_uki(arch, uki_bytes.to_vec())
        .map_err(|e| ImagerError::BuildError(format!("add UKI to raw ESP spec: {e}")))?
        .add_files(overlay_assets.to_vec())
        .map_err(|e| ImagerError::BuildError(format!("add overlay assets to raw ESP spec: {e}")))?
        .build()
        .map_err(|e| ImagerError::BuildError(format!("build raw ESP spec: {e}")))?;

    let raw_path = output_dir.join(Artifact::Raw.filename());
    let raw_path_clone = raw_path.clone();
    spawn_blocking(move || {
        let file = std::fs::File::create(&raw_path_clone).map_err(std::io::Error::other)?;
        let mut writer = std::io::BufWriter::new(file);
        miso::build_raw(&spec, &mut writer, Some(6)).map_err(std::io::Error::other)
    })
    .await
    .map_err(|e| ImagerError::BuildError(format!("join IMG build task: {e}")))?
    .map_err(|e| ImagerError::BuildError(format!("build raw disk image: {e}")))?;

    Ok(raw_path)
}

/// Write overlay boot assets to an ESP directory.
///
/// # Errors
///
/// Returns an error when creating directories or writing files fails.
pub async fn write_esp_files(files: &[esp::EspFile], esp_dir: &Path) -> Result<()> {
    for file in files {
        let dest = esp_dir.join(&file.path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| ImagerError::BuildError(format!("create esp dir: {e}")))?;
        }
        fs::write(&dest, &file.data)
            .await
            .map_err(|e| ImagerError::BuildError(format!("write esp file: {e}")))?;
    }

    Ok(())
}

fn esp_arch(arch: Arch) -> EspArch {
    match arch {
        Arch::Amd64 => EspArch::X86_64,
        Arch::Arm64 => EspArch::Aarch64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esp_arch_amd64() {
        // ARRANGE
        let arch = Arch::Amd64;

        // ACT
        let result = esp_arch(arch);

        // ASSERT
        assert_eq!(result, esp::Arch::X86_64);
    }

    #[test]
    fn esp_arch_arm64() {
        // ARRANGE
        let arch = Arch::Arm64;

        // ACT
        let result = esp_arch(arch);

        // ASSERT
        assert_eq!(result, esp::Arch::Aarch64);
    }
}
