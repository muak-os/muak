//! Per-disk partition walk over the sysfs block tree.

use std::fs;
use std::path::Path;

use super::{Roots, attrs, holders, probe};
use crate::mounts::{self, Mount};
use crate::types::Partition;

const LUKS: &str = "crypto_LUKS";

/// Discovers the partitions of `disk_name` below the sysfs block tree.
pub(crate) fn discover(
    roots: &Roots,
    sysfs_path: &Path,
    disk_name: &str,
    mounts_table: &[Mount],
) -> Vec<Partition> {
    let Ok(entries) = fs::read_dir(sysfs_path) else {
        return Vec::new();
    };

    let mut partitions = Vec::new();
    for entry in entries.flatten() {
        if let Some(partition) = try_read(roots, &entry, disk_name, mounts_table) {
            partitions.push(partition);
        }
    }

    partitions.sort_by_key(|partition| partition.number);

    partitions
}

fn try_read(
    roots: &Roots,
    entry: &fs::DirEntry,
    disk_name: &str,
    mounts_table: &[Mount],
) -> Option<Partition> {
    let name = entry.file_name();
    let name = name.to_string_lossy();

    if !name.starts_with(disk_name) || name == disk_name {
        return None;
    }

    if !entry.path().join("partition").exists() {
        return None;
    }

    read(roots, disk_name, &name, mounts_table)
}

fn read(roots: &Roots, disk_name: &str, name: &str, mounts_table: &[Mount]) -> Option<Partition> {
    let sysfs_path = roots.block.join(disk_name).join(name);

    let number = u32::try_from(attrs::read_u64(&sysfs_path.join("partition"))?).ok()?;
    let start_sector = attrs::read_u64(&sysfs_path.join("start"))?;
    let size_bytes = attrs::read_size_bytes(&sysfs_path.join("size"))?;

    let device_path = roots.dev.join(name);
    let path = device_path.to_string_lossy().into_owned();
    let fstype = fstype(roots, disk_name, name, &device_path);
    let mount_point = mounts::of(mounts_table, &path).to_owned();

    Some(Partition {
        number,
        start_sector,
        size_bytes,
        name: name.to_owned(),
        path,
        fstype,
        gpt_name: gpt_name(&sysfs_path),
        mount_point,
    })
}

fn fstype(roots: &Roots, disk_name: &str, name: &str, device_path: &Path) -> String {
    let Some(detected) = probe::filesystem(device_path) else {
        return String::new();
    };

    if detected != LUKS {
        return detected.to_owned();
    }

    mapped_fstype(roots, disk_name, name).unwrap_or_else(|| detected.to_owned())
}

fn mapped_fstype(roots: &Roots, disk_name: &str, name: &str) -> Option<String> {
    let sysfs = format!("{disk_name}/{name}");

    holders::try_names(roots, &sysfs)
        .ok()?
        .into_iter()
        .find_map(|holder| probe::filesystem(&roots.dev.join("mapper").join(holder)))
        .map(str::to_owned)
}

fn gpt_name(sysfs_path: &Path) -> String {
    let Ok(content) = fs::read_to_string(sysfs_path.join("uevent")) else {
        return String::new();
    };

    attrs::partname(&content)
}
