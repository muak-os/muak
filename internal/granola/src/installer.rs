use crate::{disk, log};
use anyhow::{Context, Result, bail};
use nix::mount::{MsFlags, mount, umount};
use nix::unistd::sync;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallationStatus {
    Live,
    Installed,
}

pub fn detect_status() -> InstallationStatus {
    if Path::new("/dev/disk/by-label/STATE").exists() {
        InstallationStatus::Installed
    } else {
        InstallationStatus::Live
    }
}

pub fn find_partition_by_label(label: &str) -> Result<String> {
    let path = format!("/dev/disk/by-label/{}", label);
    let symlink = fs::read_link(&path)
        .with_context(|| format!("Partition with label '{}' not found", label))?;

    let device = if symlink.is_absolute() {
        symlink
    } else {
        PathBuf::from("/dev/disk/by-label")
            .join(&symlink)
            .canonicalize()?
    };

    Ok(device.to_string_lossy().to_string())
}

pub fn mount_partitions() -> Result<()> {
    let state_dev = find_partition_by_label("STATE")?;
    fs::create_dir_all("/state")?;

    mount(
        Some(state_dev.as_str()),
        "/state",
        Some("btrfs"),
        MsFlags::empty(),
        None::<&str>,
    )
    .context("Failed to mount STATE partition")?;

    log!("installer", "Mounted STATE partition at /state");

    let data_dev = find_partition_by_label("DATA")?;
    fs::create_dir_all("/var")?;

    mount(
        Some(data_dev.as_str()),
        "/var",
        Some("btrfs"),
        MsFlags::empty(),
        None::<&str>,
    )
    .context("Failed to mount DATA partition")?;

    log!("installer", "Mounted DATA partition at /var");

    Ok(())
}

pub fn install(disk_path: &str, force: bool) -> Result<()> {
    log!("installer", "Starting installation to {}", disk_path);

    if detect_status() != InstallationStatus::Live {
        bail!("Cannot install from an already-installed system. Boot from live ISO.");
    }

    if !Path::new(disk_path).exists() {
        bail!("Disk '{}' does not exist", disk_path);
    }

    disk::validate_block_device(disk_path)?;
    disk::validate_disk_size(disk_path)?;

    if !force {
        if disk::has_existing_partitions(disk_path)? {
            bail!(
                "Disk '{}' has existing partitions. Use --force to overwrite.",
                disk_path
            );
        }
    }

    disk::wipe_disk(disk_path)?;

    let (efi_part, state_part, data_part) = disk::create_partitions(disk_path)?;

    disk::format_efi_partition(&efi_part)?;
    disk::format_btrfs_partition(&state_part, "STATE")?;
    disk::format_btrfs_partition(&data_part, "DATA")?;

    install_uki(&efi_part)?;

    initialize_state_partition(&state_part)?;

    log!("installer", "Installation completed successfully!");
    log!(
        "installer",
        "Remove the ISO and reboot to start from installed disk."
    );

    // TODO: send reboot to kernel

    Ok(())
}

fn install_uki(efi_device: &str) -> Result<()> {
    log!("installer", "Installing UKI to EFI partition");

    let arch = std::env::consts::ARCH;
    let uki_filename = match arch {
        "x86_64" => "BOOTX64.EFI",
        "aarch64" => "BOOTAA64.EFI",
        _ => bail!("Unsupported architecture: {}", arch),
    };

    let source_uki = find_uki_on_live_media()?;

    let mount_point = "/mnt/efi";
    fs::create_dir_all(mount_point)?;

    mount(
        Some(efi_device),
        mount_point,
        Some("vfat"),
        MsFlags::MS_NOATIME,
        None::<&str>,
    )
    .context("Failed to mount EFI partition")?;

    let result = (|| -> Result<()> {
        fs::create_dir_all(format!("{}/EFI/BOOT", mount_point))?;

        let dest_uki = format!("{}/EFI/BOOT/{}", mount_point, uki_filename);
        fs::copy(&source_uki, &dest_uki)
            .with_context(|| format!("Failed to copy UKI from {} to {}", source_uki, dest_uki))?;

        log!("installer", "Copied {} to {}", source_uki, dest_uki);

        sync();
        Ok(())
    })();

    if let Err(e) = umount(mount_point) {
        log!(
            "installer",
            "Warning: Failed to unmount {}: {}",
            mount_point,
            e
        );
    }

    result?;

    log!("installer", "UKI installation complete");

    Ok(())
}

fn find_uki_on_live_media() -> Result<String> {
    let candidates = vec!["/run/uki/BOOTX64.EFI", "/run/uki/BOOTAA64.EFI"];

    for candidate in &candidates {
        if Path::new(candidate).exists() {
            log!("installer", "Found UKI at {}", candidate);
            return Ok(candidate.to_string());
        }
    }

    bail!(
        "Could not find UKI in /run/uki/. Stage 1 init should have copied it there. Searched: {:?}",
        candidates
    )
}

// TODO: understand what config files are needed and populate them
fn initialize_state_partition(device: &str) -> Result<()> {
    log!("installer", "Initializing STATE partition");

    let mount_point = "/mnt/state";
    fs::create_dir_all(mount_point)?;

    mount(
        Some(device),
        mount_point,
        Some("btrfs"),
        MsFlags::empty(),
        None::<&str>,
    )
    .context("Failed to mount STATE partition")?;

    let result = (|| -> Result<()> {
        fs::create_dir_all(format!("{}/config", mount_point))?;
        fs::create_dir_all(format!("{}/network", mount_point))?;

        let default_config = "# Muak Configuration\n# TODO: Add actual config\n";
        fs::write(
            format!("{}/config/config.yaml", mount_point),
            default_config,
        )?;

        sync();
        Ok(())
    })();

    if let Err(e) = umount(mount_point) {
        log!(
            "installer",
            "Warning: Failed to unmount {}: {}",
            mount_point,
            e
        );
    }

    result?;

    log!("installer", "STATE partition initialized");

    Ok(())
}
