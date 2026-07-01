//! Bootable media builders.

use std::io::Write;
use std::os::unix::net::UnixStream;

use esp::model::Arch as EspArch;
use esp::model::EspFile;
use esp::model::EspSpec;
use koci::arch::Arch;

use crate::error::{Result, WizardError};
use crate::resolve::BuildPlan;

/// Build an ISO image, writing directly to a `Write` sink.
///
/// # Errors
///
/// Returns an error when creating the ISO or writing it fails.
pub fn iso_to_writer<W: Write>(
    resolved_profile: &BuildPlan,
    uki_reader: UnixStream,
    uki_size: u64,
    writer: &mut W,
) -> Result<()> {
    let arch = esp_arch(resolved_profile.arch());
    let boot = EspFile::boot(arch, uki_reader, uki_size);
    let mut spec = EspSpec::builder()
        .add_file(boot)
        .map_err(|e| WizardError::BuildError(format!("add UKI to ISO ESP spec: {e}")))?
        .build()
        .map_err(|e| WizardError::BuildError(format!("build ISO ESP spec: {e}")))?;

    miso::build_iso(&mut spec, writer)
        .map_err(|e| WizardError::BuildError(format!("build bootable ISO: {e}")))?;

    Ok(())
}

/// Build a raw disk image, writing directly to a `Write` sink.
///
/// # Errors
///
/// Returns an error when creating the raw image or writing it fails.
pub fn raw_to_writer<W: Write>(
    resolved_profile: &BuildPlan,
    overlay_assets: Vec<EspFile>,
    uki_reader: UnixStream,
    uki_size: u64,
    writer: &mut W,
) -> Result<()> {
    let arch = esp_arch(resolved_profile.arch());
    let boot = EspFile::boot(arch, uki_reader, uki_size);

    let mut all: Vec<EspFile> = Vec::with_capacity(overlay_assets.len().saturating_add(1));
    all.push(boot);
    all.extend(overlay_assets);
    let mut spec = EspSpec::builder()
        .add_files(all)
        .map_err(|e| WizardError::BuildError(format!("add UKI to raw ESP spec: {e}")))?
        .build()
        .map_err(|e| WizardError::BuildError(format!("build raw ESP spec: {e}")))?;

    miso::build_raw(&mut spec, writer, Some(6))
        .map_err(|e| WizardError::BuildError(format!("build raw disk image: {e}")))?;

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
        assert_eq!(result, EspArch::X86_64);
    }

    #[test]
    fn esp_arch_arm64() {
        // ARRANGE
        let arch = Arch::Arm64;

        // ACT
        let result = esp_arch(arch);

        // ASSERT
        assert_eq!(result, EspArch::Aarch64);
    }
}
