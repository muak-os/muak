use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use rustix::fs::sync;
use rustix::mount::{MountFlags, mount};
use sysconfig::HostConfig;

use crate::disk;

use super::uki;
use super::{
    INSTALL_DIR, InstallationStatus, mount_efi_partition, prepare_uki, status, unmount_partition,
};
use sysconfig;

pub fn install(disk_path: &str, force: bool, config: &HostConfig) -> Result<()> {
    kmsg::info!(@ "provisioning", "Starting installation to {}", disk_path);

    validate(disk_path, force)?;

    let work_dir = Path::new(INSTALL_DIR);
    let components = prepare_uki(&config.system.image, &config.system.extensions, work_dir)?;
    let staged_uki = work_dir.join("staged.efi");

    uki::build(&components, &staged_uki)?;

    disk::delete_all_partitions_blkpg(disk_path)?;
    disk::wipe_disk(disk_path)?;
    let (efi_part, state_part, data_part) = disk::create_partitions(disk_path)?;

    disk::format_efi_partition(&efi_part)?;
    disk::format_btrfs_partition(&state_part, "STATE")?;
    disk::format_btrfs_partition(&data_part, "DATA")?;

    deploy_uki_to_efi(&efi_part, &staged_uki)?;
    init_state_partition(&state_part, config)?;

    if let Err(e) = uki::cleanup_dir(work_dir) {
        kmsg::warn!(@ "provisioning", "Failed to cleanup work dir: {}", e);
    }

    sync();
    kmsg::info!(@ "provisioning", "Installation completed successfully!");

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

    let mounted = disk::get_disk_mounts(disk_path);
    if !mounted.is_empty() && !force {
        bail!(
            "Cannot install: {} is mounted at {}. Use --force to unmount automatically.",
            mounted[0].device,
            mounted[0].mount_point
        );
    }
    sync();
    disk::unmount_all(&mounted)?;

    if !force && disk::has_existing_partitions(disk_path)? {
        bail!(
            "Disk '{}' has existing partitions. Use --force to overwrite.",
            disk_path
        );
    }

    Ok(())
}

fn deploy_uki_to_efi(efi_device: &str, staged_uki: &Path) -> Result<()> {
    if !Path::new(efi_device).exists() {
        bail!("EFI device {} does not exist", efi_device);
    }

    let mount_point = "/run/mnt/efi";
    mount_efi_partition(efi_device, mount_point)?;

    let result = write_uki_to_efi(mount_point, staged_uki);

    unmount_partition(mount_point);

    result?;
    kmsg::info!(@ "provisioning", "UKI deployed to EFI partition");
    Ok(())
}

fn write_uki_to_efi(mount_point: &str, staged_uki: &Path) -> Result<()> {
    fs::create_dir_all(format!("{}/EFI/BOOT", mount_point))?;

    let uki_path = uki::get_uki_path(Path::new(mount_point))?;
    fs::copy(staged_uki, &uki_path)
        .with_context(|| format!("Failed to copy UKI to {}", uki_path.display()))?;

    sync();
    Ok(())
}

fn init_state_partition(device: &str, config: &HostConfig) -> Result<()> {
    kmsg::info!(@ "provisioning", "Initializing STATE partition");

    let mount_point = "/run/mnt/state";

    fs::create_dir_all(mount_point)
        .with_context(|| format!("Failed to create mount point {}", mount_point))?;

    mount(device, mount_point, "btrfs", MountFlags::empty(), None)
        .context("Failed to mount STATE partition")?;

    let config_toml = toml::to_string_pretty(config).context("Failed to serialize config")?;
    fs::write(format!("{}/config.toml", mount_point), config_toml)
        .context("Failed to write config.toml")?;

    sync();
    unmount_partition(mount_point);

    kmsg::info!(@ "provisioning", "STATE partition initialized");
    Ok(())
}
