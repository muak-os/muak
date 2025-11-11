use crate::{disk, log};
use anyhow::{Context, Result, bail};
use nix::mount::{MsFlags, mount, umount};
// use nix::sys::reboot::{LINUX_REBOOT_CMD_POWER_OFF, LINUX_REBOOT_CMD_RESTART, reboot};
use nix::unistd::sync;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallationStatus {
    Live,
    Installed,
}

pub fn detect_status() -> InstallationStatus {
    if Path::new("/dev/disk/by-label/STATE").exists() {
        return InstallationStatus::Installed;
    }

    // Fallback: probe partitions without relying on udev by-label symlinks
    if probe_state_device().is_some() {
        InstallationStatus::Installed
    } else {
        InstallationStatus::Live
    }
}

fn probe_state_device() -> Option<String> {
    probe_device_by_label("STATE")
}

fn probe_device_by_label(label: &str) -> Option<String> {
    // Try by-partlabel symlink first
    let by_partlabel = format!("/dev/disk/by-partlabel/{}", label);
    if let Ok(symlink) = fs::read_link(&by_partlabel) {
        let device = if symlink.is_absolute() {
            symlink
        } else {
            PathBuf::from("/dev/disk/by-partlabel").join(&symlink)
        };
        if let Ok(canon) = device.canonicalize() {
            return Some(canon.to_string_lossy().to_string());
        }
    }

    // Scan sysfs uevent for PARTNAME
    if let Ok(entries) = fs::read_dir("/sys/class/block") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // Only consider partitions (have a 'partition' file)
            let part_flag = entry.path().join("partition");
            if !part_flag.exists() {
                continue;
            }
            let uevent = entry.path().join("uevent");
            if let Ok(content) = fs::read_to_string(&uevent) {
                for line in content.lines() {
                    if line.trim() == format!("PARTNAME={}", label) {
                        let dev_path = format!("/dev/{}", name);
                        if Path::new(&dev_path).exists() {
                            return Some(dev_path);
                        }
                    }
                }
            }
        }
    }

    None
}

pub fn find_partition_by_label(label: &str) -> Result<String> {
    let path = format!("/dev/disk/by-label/{}", label);
    if let Ok(symlink) = fs::read_link(&path) {
        let device = if symlink.is_absolute() {
            symlink
        } else {
            PathBuf::from("/dev/disk/by-label").join(&symlink).canonicalize()?
        };
        return Ok(device.to_string_lossy().to_string());
    }

    // Fallback to probing by partlabel/sysfs
    if let Some(dev) = probe_device_by_label(label) {
        return Ok(dev);
    }

    bail!("Partition with label '{}' not found", label)
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
    // Future: take an option to auto poweroff or reboot after install.
    // For now we just sync at end; caller can trigger reboot.

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

    sync();

    log!("installer", "Installation completed successfully!");
    log!(
        "installer",
        "Remove the ISO and reboot to start from installed disk."
    );

    // TODO: reboot

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

    log!("installer", "EFI device: {}", efi_device);
    log!("installer", "Checking if EFI device exists...");
    if !Path::new(efi_device).exists() {
        bail!("EFI device {} does not exist", efi_device);
    }
    log!("installer", "EFI device exists");

    let mount_point = "/mnt/efi";
    log!("installer", "Creating mount point: {}", mount_point);
    fs::create_dir_all(mount_point)
        .with_context(|| format!("Failed to create mount point {}", mount_point))?;
    log!("installer", "Mount point created");

    log!(
        "installer",
        "Attempting to mount {} at {}",
        efi_device,
        mount_point
    );
    mount(
        Some(efi_device),
        mount_point,
        Some("vfat"),
        MsFlags::MS_NOATIME,
        None::<&str>,
    )
    .with_context(|| {
        format!(
            "Failed to mount EFI partition {} at {}",
            efi_device, mount_point
        )
    })?;
    log!("installer", "EFI partition mounted successfully");

    let result = (|| -> Result<()> {
        fs::create_dir_all(format!("{}/EFI/BOOT", mount_point))?;

        let dest_uki = format!("{}/EFI/BOOT/{}", mount_point, uki_filename);

        build_uki(&dest_uki)?;

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

fn build_uki(output_path: &str) -> Result<()> {
    let stub_path = "/run/uki/stub.efi";
    let kernel_path = "/run/uki/bzImage";
    let initrd_path = "/run/uki/initrd.img";
    let cmdline_path = "/run/uki/cmdline.txt";

    if !Path::new(stub_path).exists() {
        bail!(
            "Stub binary not found at {}. Cannot build UKI on-the-fly.",
            stub_path
        );
    }
    if !Path::new(kernel_path).exists() {
        bail!(
            "Kernel not found at {}. Cannot build UKI on-the-fly.",
            kernel_path
        );
    }
    if !Path::new(initrd_path).exists() {
        bail!(
            "Initrd not found at {}. Cannot build UKI on-the-fly.",
            initrd_path
        );
    }
    if !Path::new(cmdline_path).exists() {
        bail!(
            "Cmdline not found at {}. Cannot build UKI on-the-fly.",
            cmdline_path
        );
    }

    let output = Command::new("/bin/yuki")
        .arg("--stub")
        .arg(stub_path)
        .arg("--linux")
        .arg(kernel_path)
        .arg("--initrd")
        .arg(initrd_path)
        .arg("--cmdline")
        .arg(cmdline_path)
        .arg("--output")
        .arg(output_path)
        .output()
        .context("Failed to execute yuki")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("yuki failed to build UKI: {}", stderr);
    }

    log!("installer", "Successfully built UKI at {}", output_path);

    Ok(())
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
