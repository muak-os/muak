//! Per-disk partition walk over the sysfs block tree.

use std::fs;
use std::path::Path;

use parttable::gpt;

use crate::mounts::{self, Mount};
use crate::roots::Roots;
use crate::types::Partition;
use crate::{attrs, probe};

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
    let size_sectors = attrs::read_u64(&sysfs_path.join("size"))?;
    let size_bytes = size_sectors.saturating_mul(gpt::plan::SECTOR_SIZE);

    let device_path = roots.dev.join(name);
    let path = device_path.to_string_lossy().into_owned();
    let fstype = probe::filesystem(&device_path)
        .unwrap_or_default()
        .to_owned();
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

fn gpt_name(sysfs_path: &Path) -> String {
    let Ok(content) = fs::read_to_string(sysfs_path.join("uevent")) else {
        return String::new();
    };

    attrs::partname(&content)
}
