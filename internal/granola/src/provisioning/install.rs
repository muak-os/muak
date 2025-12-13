use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use nix::mount::{MsFlags, mount};
use nix::unistd::sync;

use crate::{disk, log};

use super::uki;
use super::{
    INSTALL_WORK_DIR, InstallationStatus, mount_efi_partition, prepare_uki, status,
    unmount_partition,
};

pub fn install(disk_path: &str, force: bool, version: &str, extensions: &[String]) -> Result<()> {
    log!("provisioning", "Starting installation to {}", disk_path);

    validate(disk_path, force)?;

    disk::delete_all_partitions_blkpg(disk_path)?;
    disk::wipe_disk(disk_path)?;
    let (efi_part, state_part, data_part) = disk::create_partitions(disk_path)?;

    disk::format_efi_partition(&efi_part)?;
    disk::format_btrfs_partition(&state_part, "STATE")?;
    disk::format_btrfs_partition(&data_part, "DATA")?;

    let installer_image = format!("ghcr.io/sawangg/installer:{}", version);
    deploy_uki_to_efi(&efi_part, &installer_image, extensions)?;

    init_state_partition(&state_part, version)?;

    if let Err(e) = uki::cleanup_dir(Path::new(INSTALL_WORK_DIR)) {
        log!("provisioning", "Warning: Failed to cleanup work dir: {}", e);
    }

    sync();
    log!("provisioning", "Installation completed successfully!");

    Ok(())
}

fn validate(disk_path: &str, force: bool) -> Result<()> {
    if !force && status() != InstallationStatus::Live {
        bail!(
            "Cannot install from an already-installed system. Boot from live ISO or use --force."
        );
    }

    if !Path::new(disk_path).exists() {
        bail!("Disk '{}' does not exist", disk_path);
    }

    disk::validate_block_device(disk_path)?;
    disk::validate_disk_size(disk_path)?;
    disk::check_disk_not_mounted(disk_path)?;

    if !force && disk::has_existing_partitions(disk_path)? {
        bail!(
            "Disk '{}' has existing partitions. Use --force to overwrite.",
            disk_path
        );
    }

    Ok(())
}

fn deploy_uki_to_efi(efi_device: &str, installer_image: &str, extensions: &[String]) -> Result<()> {
    if !Path::new(efi_device).exists() {
        bail!("EFI device {} does not exist", efi_device);
    }

    let mount_point = "/mnt/efi";
    mount_efi_partition(efi_device, mount_point)?;

    let result = build_and_install_uki(mount_point, installer_image, extensions);

    unmount_partition(mount_point);

    result?;
    log!("provisioning", "UKI deployed to EFI partition");
    Ok(())
}

fn build_and_install_uki(
    mount_point: &str,
    installer_image: &str,
    extensions: &[String],
) -> Result<()> {
    fs::create_dir_all(format!("{}/EFI/BOOT", mount_point))?;

    let components = prepare_uki(installer_image, extensions, Path::new(INSTALL_WORK_DIR))?;
    let uki_path = uki::get_uki_path(Path::new(mount_point))?;
    uki::build_uki(&components, &uki_path)?;

    sync();
    Ok(())
}

fn init_state_partition(device: &str, version: &str) -> Result<()> {
    log!("provisioning", "Initializing STATE partition");

    let mount_point = "/mnt/state";

    fs::create_dir_all(mount_point)
        .with_context(|| format!("Failed to create mount point {}", mount_point))?;

    mount(
        Some(device),
        mount_point,
        Some("btrfs"),
        MsFlags::empty(),
        None::<&str>,
    )
    .context("Failed to mount STATE partition")?;

    fs::create_dir_all(format!("{}/config", mount_point))?;
    fs::write(format!("{}/VERSION", mount_point), version)
        .context("Failed to write VERSION file")?;

    sync();
    unmount_partition(mount_point);

    log!("provisioning", "STATE partition initialized");
    Ok(())
}
