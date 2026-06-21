//! Bootable media builders (ISO, raw disk image, ESP overlay).

use std::io::Write;

use esp::Arch as EspArch;
use esp::EspSpecBuilder;
use koci::arch::Arch;
use tokio::task::spawn_blocking;

use crate::error::{WizardError, Result};
use crate::resolve::ResolvedProfile;

/// Build an ISO image, writing to a `Write` sink.
///
/// # Errors
///
/// Returns an error when creating the ISO or writing it fails.
pub async fn iso_to_writer<W: Write>(
    resolved_profile: &ResolvedProfile,
    uki_bytes: &[u8],
    writer: &mut W,
) -> Result<()> {
    let arch = esp_arch(resolved_profile.arch());
    let spec = EspSpecBuilder::default()
        .with_uki(arch, uki_bytes.to_vec())
        .map_err(|e| WizardError::BuildError(format!("add UKI to ISO ESP spec: {e}")))?
        .build()
        .map_err(|e| WizardError::BuildError(format!("build ISO ESP spec: {e}")))?;

    let buf = spawn_blocking(move || {
        let mut buf = Vec::new();
        miso::build_iso(&spec, &mut buf).map_err(std::io::Error::other)?;
        Ok::<_, std::io::Error>(buf)
    })
    .await
    .map_err(|e| WizardError::BuildError(format!("join ISO build task: {e}")))?
    .map_err(|e| WizardError::BuildError(format!("build bootable ISO: {e}")))?;

    writer
        .write_all(&buf)
        .map_err(|e| WizardError::BuildError(format!("write ISO: {e}")))?;

    Ok(())
}

/// Build a raw disk image, writing to a `Write` sink.
///
/// # Errors
///
/// Returns an error when creating the raw image or writing it fails.
pub async fn raw_to_writer<W: Write>(
    resolved_profile: &ResolvedProfile,
    overlay_assets: &[esp::EspFile],
    uki_bytes: &[u8],
    writer: &mut W,
) -> Result<()> {
    let arch = esp_arch(resolved_profile.arch());
    let spec = EspSpecBuilder::default()
        .with_uki(arch, uki_bytes.to_vec())
        .map_err(|e| WizardError::BuildError(format!("add UKI to raw ESP spec: {e}")))?
        .add_files(overlay_assets.to_vec())
        .map_err(|e| WizardError::BuildError(format!("add overlay assets to raw ESP spec: {e}")))?
        .build()
        .map_err(|e| WizardError::BuildError(format!("build raw ESP spec: {e}")))?;

    let buf = spawn_blocking(move || {
        let mut buf = Vec::new();
        miso::build_raw(&spec, &mut buf, Some(6)).map_err(std::io::Error::other)?;
        Ok::<_, std::io::Error>(buf)
    })
    .await
    .map_err(|e| WizardError::BuildError(format!("join IMG build task: {e}")))?
    .map_err(|e| WizardError::BuildError(format!("build raw disk image: {e}")))?;

    writer
        .write_all(&buf)
        .map_err(|e| WizardError::BuildError(format!("write raw image: {e}")))?;

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
