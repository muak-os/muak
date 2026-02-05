use std::path::PathBuf;

use anyhow::{Context, Result};
use rustix::fs::{Mode, OFlags, open};
use rustix::ioctl::{Opcode, Setter, ioctl, opcode};

use super::DATA_DIR;

const BTRFS_IOCTL_MAGIC: u8 = 0x94;
const BTRFS_PATH_NAME_MAX: usize = 4087;
const BTRFS_IOC_SUBVOL_CREATE: Opcode = opcode::write::<VolArgs>(BTRFS_IOCTL_MAGIC, 14);
const BTRFS_IOC_SNAP_DESTROY: Opcode = opcode::write::<VolArgs>(BTRFS_IOCTL_MAGIC, 15);

/// Represents the btrfs_ioctl_vol_args structure from kernel
#[repr(C)]
struct VolArgs {
    fd: i64,
    name: [u8; BTRFS_PATH_NAME_MAX + 1],
}

/// Create a Btrfs subvolume for a specific vm
pub fn create_subvolume(vm_id: &str) -> Result<PathBuf> {
    let path = PathBuf::from(DATA_DIR).join(vm_id);

    let file = open(DATA_DIR, OFlags::RDONLY | OFlags::DIRECTORY, Mode::empty())
        .context("Failed to open data directory")?;

    let mut args = VolArgs {
        fd: -1,
        name: [0u8; BTRFS_PATH_NAME_MAX + 1],
    };

    let name_bytes = vm_id.as_bytes();
    let copy_len = name_bytes.len().min(BTRFS_PATH_NAME_MAX);
    args.name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);

    // SAFETY: ioctl is inherently unsafe, but Setter ensures proper argument passing
    unsafe { ioctl(&file, Setter::<BTRFS_IOC_SUBVOL_CREATE, VolArgs>::new(args)) }
        .map_err(|e| anyhow::anyhow!("Failed to create subvolume {}: {}", path.display(), e))?;

    kmsg::info!(@ "vmd", "Created btrfs subvolume at {}", path.display());
    Ok(path)
}

/// Delete a Btrfs subvolume for a specific vm
pub fn delete_subvolume(vm_id: &str) -> Result<()> {
    let path = PathBuf::from(DATA_DIR).join(vm_id);

    if !path.exists() {
        return Ok(());
    }

    let file = open(DATA_DIR, OFlags::RDONLY | OFlags::DIRECTORY, Mode::empty())
        .context("Failed to open data directory")?;

    let mut args = VolArgs {
        fd: -1,
        name: [0u8; BTRFS_PATH_NAME_MAX + 1],
    };

    let name_bytes = vm_id.as_bytes();
    let copy_len = name_bytes.len().min(BTRFS_PATH_NAME_MAX);
    args.name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);

    // SAFETY: ioctl is inherently unsafe, but Setter ensures proper argument passing
    unsafe { ioctl(&file, Setter::<BTRFS_IOC_SNAP_DESTROY, VolArgs>::new(args)) }
        .map_err(|e| anyhow::anyhow!("Failed to delete subvolume {}: {}", path.display(), e))?;

    kmsg::info!(@ "vmd", "Deleted btrfs subvolume at {}", path.display());
    Ok(())
}

/// List subvolumes by reading directories in DATA_DIR
pub fn list_subvolumes() -> Result<Vec<String>> {
    let mut vm_ids = Vec::new();

    let entries = std::fs::read_dir(DATA_DIR).context("Failed to read data directory")?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir()
            && let Some(name) = path.file_name()
            && let Some(name_str) = name.to_str()
            && !name_str.starts_with('.')
        {
            vm_ids.push(name_str.to_string());
        }
    }

    Ok(vm_ids)
}
