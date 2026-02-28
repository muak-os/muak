//! EFI partition deployment of the Unified Kernel Image.

use std::path::Path;

use anyhow::{Context, Result, bail};
use rustix::fs::sync;

use crate::disk;
use crate::uki;

/// Mount point for the EFI partition during deployment.
const MOUNT_POINT: &str = "/run/mnt/efi";

/// Deploys the staged UKI to the EFI partition.
pub fn deploy(efi_device: &str, staged_uki: &Path) -> Result<()> {
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

    sync();
    disk::try_unmount(MOUNT_POINT);

    Ok(())
}
