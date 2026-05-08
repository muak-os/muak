//! EFI partition deployment of the Unified Kernel Image and board firmware.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rustix::fs::sync;

use crate::constants::host_oci_arch;
use crate::disk;
use crate::uki;

/// Mount point for the EFI partition during deployment.
const MOUNT_POINT: &str = "/run/mnt/efi";

/// Pulls a board firmware OCI image and returns the variant subdirectory path.
pub async fn pull_firmware(firmware_ref: &str, variant: &str, dest: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(dest)
        .with_context(|| format!("Failed to create firmware dir {}", dest.display()))?;

    imager::pull(firmware_ref, host_oci_arch(), dest, None)
        .await
        .with_context(|| format!("Failed to pull board firmware: {firmware_ref}"))?;

    let variant_dir = dest.join(variant);
    if !variant_dir.is_dir() {
        bail!(
            "Firmware variant '{}' not found in {}",
            variant,
            dest.display()
        );
    }

    Ok(variant_dir)
}

/// Resolves the firmware directory from host config, pulling the OCI image if configured.
pub async fn resolve_firmware(host: &config::HostConfig, dest: &Path) -> Result<Option<PathBuf>> {
    match (&host.firmware, &host.firmware_variant) {
        (Some(firmware_ref), Some(variant)) => {
            let dir = pull_firmware(firmware_ref, variant, dest).await?;
            Ok(Some(dir))
        }
        (None, None) => Ok(None),
        _ => bail!("host.firmware and host.firmware_variant must both be set or both be omitted"),
    }
}

/// Deploys the staged UKI and optional board firmware to the EFI partition.
pub fn deploy(efi_device: &str, staged_uki: &Path, firmware_dir: Option<&Path>) -> Result<()> {
    if !Path::new(efi_device).exists() {
        bail!("EFI device {} does not exist", efi_device);
    }

    disk::mount_efi_partition(efi_device, MOUNT_POINT)?;

    std::fs::create_dir_all(format!("{}/EFI/BOOT", MOUNT_POINT))?;

    let uki_path = Path::new(MOUNT_POINT)
        .join("EFI")
        .join("BOOT")
        .join(uki::UKI_FILENAME);
    std::fs::copy(staged_uki, &uki_path)
        .with_context(|| format!("Failed to copy UKI to {}", uki_path.display()))?;

    if let Some(dir) = firmware_dir {
        copy_firmware(dir, Path::new(MOUNT_POINT))?;
    }

    sync();
    disk::try_unmount(MOUNT_POINT);

    Ok(())
}

/// Copies all files from the firmware directory to the EFI partition root.
fn copy_firmware(src: &Path, efi_root: &Path) -> Result<()> {
    for entry in std::fs::read_dir(src)
        .with_context(|| format!("Failed to read firmware dir {}", src.display()))?
    {
        let entry = entry.context("Failed to read firmware entry")?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name();
        let dest = efi_root.join(&name);
        std::fs::copy(&path, &dest).with_context(|| {
            format!(
                "Failed to copy firmware file {} to {}",
                path.display(),
                dest.display()
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_firmware_copies_files_to_dest() {
        // ARRANGE
        let src = tempfile::tempdir().expect("create src dir");
        let dst = tempfile::tempdir().expect("create dst dir");
        std::fs::write(src.path().join("start4.elf"), b"gpu-fw").expect("write start4");
        std::fs::write(src.path().join("config.txt"), b"kernel=u-boot.bin").expect("write config");
        std::fs::create_dir(src.path().join("subdir")).expect("create subdir");

        // ACT
        copy_firmware(src.path(), dst.path()).expect("copy firmware");

        // ASSERT
        assert_eq!(
            std::fs::read(dst.path().join("start4.elf")).expect("read"),
            b"gpu-fw"
        );
        assert_eq!(
            std::fs::read(dst.path().join("config.txt")).expect("read"),
            b"kernel=u-boot.bin"
        );
        assert!(!dst.path().join("subdir").exists());
    }

    #[test]
    fn copy_firmware_empty_dir_is_noop() {
        // ARRANGE
        let src = tempfile::tempdir().expect("create src dir");
        let dst = tempfile::tempdir().expect("create dst dir");

        // ACT
        copy_firmware(src.path(), dst.path()).expect("copy firmware");

        // ASSERT
        let count = std::fs::read_dir(dst.path()).expect("read dst").count();
        assert_eq!(count, 0);
    }

    #[test]
    fn copy_firmware_fails_on_missing_src() {
        // ARRANGE
        let dst = tempfile::tempdir().expect("create dst dir");

        // ACT
        let result = copy_firmware(Path::new("/nonexistent/firmware"), dst.path());

        // ASSERT
        assert!(result.is_err());
    }
}
