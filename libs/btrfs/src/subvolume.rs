//! Subvolume management for btrfs filesystems.

use std::path::PathBuf;

use rustix::fs::{Mode, OFlags, open};
use rustix::ioctl::{Setter, ioctl};

use crate::error::{BtrfsError, Result};
use crate::ioctl::{BTRFS_IOC_SNAP_DESTROY, BTRFS_IOC_SUBVOL_CREATE, BTRFS_PATH_NAME_MAX, VolArgs};

/// Create a Btrfs subvolume for a specific VM.
///
/// # Arguments
/// * `vm_id` - The VM/subvolume identifier
/// * `data_dir` - Data directory where subvolume will be created
///
/// # Returns
/// The path to the created subvolume on success.
pub fn create_subvolume(vm_id: &str, data_dir: &str) -> Result<PathBuf> {
    let path = PathBuf::from(data_dir).join(vm_id);

    let file = open(data_dir, OFlags::RDONLY | OFlags::DIRECTORY, Mode::empty())?;

    let mut args = VolArgs {
        fd: -1,
        name: [0u8; BTRFS_PATH_NAME_MAX + 1],
    };

    let name_bytes = vm_id.as_bytes();
    let copy_len = name_bytes.len().min(BTRFS_PATH_NAME_MAX);
    args.name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);

    // SAFETY: ioctl is inherently unsafe, but Setter ensures proper argument passing
    unsafe { ioctl(&file, Setter::<BTRFS_IOC_SUBVOL_CREATE, VolArgs>::new(args)) }.map_err(
        |source| BtrfsError::Subvolume {
            operation: "create".to_string(),
            path: path.clone(),
            source,
        },
    )?;

    Ok(path)
}

/// Delete a Btrfs subvolume for a specific VM.
///
/// # Arguments
/// * `vm_id` - The VM/subvolume identifier
/// * `data_dir` - Data directory where subvolume is located
///
/// # Errors
/// Returns an error if the ioctl fails.
pub fn delete_subvolume(vm_id: &str, data_dir: &str) -> Result<()> {
    let path = PathBuf::from(data_dir).join(vm_id);

    if !path.exists() {
        return Ok(());
    }

    let file = open(data_dir, OFlags::RDONLY | OFlags::DIRECTORY, Mode::empty())?;

    let mut args = VolArgs {
        fd: -1,
        name: [0u8; BTRFS_PATH_NAME_MAX + 1],
    };

    let name_bytes = vm_id.as_bytes();
    let copy_len = name_bytes.len().min(BTRFS_PATH_NAME_MAX);
    args.name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);

    // SAFETY: ioctl is inherently unsafe, but Setter ensures proper argument passing
    unsafe { ioctl(&file, Setter::<BTRFS_IOC_SNAP_DESTROY, VolArgs>::new(args)) }.map_err(
        |source| BtrfsError::Subvolume {
            operation: "delete".to_string(),
            path: path.clone(),
            source,
        },
    )?;

    Ok(())
}

/// List subvolumes by reading directories in the data directory.
///
/// # Arguments
/// * `data_dir` - Data directory to list subvolumes from
///
/// # Returns
/// A vector of subvolume names.
pub fn list_subvolumes(data_dir: &str) -> Result<Vec<String>> {
    let mut vm_ids = Vec::new();

    let entries = std::fs::read_dir(data_dir)?;

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
