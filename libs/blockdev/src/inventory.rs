//! Physical disk inventory read from the sysfs block tree.

use std::fs;
use std::path::Path;

use parttable::gpt;

use crate::error::{BlockdevError, Result};
use crate::mounts::{self, Mount};
use crate::roots::Roots;
use crate::types::Disk;
use crate::{attrs, partitions};

/// Lists the physical disks reported by the running kernel.
///
/// # Errors
///
/// Returns an error when `/sys/block` is missing or cannot be read, which
/// usually means sysfs is not mounted.
pub fn list() -> Result<Vec<Disk>> {
    list_in(&Roots::system())
}

/// Lists the physical disks reported below injectable roots.
///
/// # Errors
///
/// Returns an error when the block root is missing or cannot be read.
pub fn list_in(roots: &Roots) -> Result<Vec<Disk>> {
    if !roots.block.exists() {
        return Err(BlockdevError::MissingRoot(roots.block.clone()));
    }

    let mounts_table = mounts::read(&roots.mounts);
    let mut disks = Vec::new();

    for entry in fs::read_dir(&roots.block)?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if let Some(disk) = try_read(roots, &name, &mounts_table) {
            disks.push(disk);
        }
    }

    disks.sort_by(|left, right| left.name.cmp(&right.name));

    Ok(disks)
}

fn try_read(roots: &Roots, name: &str, mounts_table: &[Mount]) -> Option<Disk> {
    if !is_physical_disk(name) {
        return None;
    }

    read(roots, name, mounts_table).filter(|disk| disk.size_bytes > 0)
}

fn read(roots: &Roots, name: &str, mounts_table: &[Mount]) -> Option<Disk> {
    let sysfs_path = roots.block.join(name);

    let size_sectors = attrs::read_u64(&sysfs_path.join("size"))?;
    let size_bytes = size_sectors.saturating_mul(gpt::plan::SECTOR_SIZE);
    let removable = attrs::read_u64(&sysfs_path.join("removable"))? != 0;
    let read_only = attrs::read_u64(&sysfs_path.join("ro"))? != 0;

    let model =
        attrs::read_string(&sysfs_path.join("device/model")).unwrap_or_else(|| name.to_owned());
    let serial = read_fact(&sysfs_path, "device/serial", "serial");
    let wwid = read_fact(&sysfs_path, "wwid", "device/wwid");
    let rotational = attrs::read_u64(&sysfs_path.join("queue/rotational")).unwrap_or(0) != 0;
    let bus_path = bus_path(roots, &sysfs_path);
    let subsystem = attrs::read_link_name(&sysfs_path.join("device/subsystem")).unwrap_or_default();

    Some(Disk {
        name: name.to_owned(),
        path: roots.dev.join(name).to_string_lossy().into_owned(),
        size_bytes,
        model,
        serial,
        wwid,
        bus_path,
        subsystem,
        rotational,
        removable,
        read_only,
        partitions: partitions::discover(roots, &sysfs_path, name, mounts_table),
    })
}

fn read_fact(sysfs_path: &Path, primary: &str, fallback: &str) -> String {
    attrs::read_string(&sysfs_path.join(primary))
        .or_else(|| attrs::read_string(&sysfs_path.join(fallback)))
        .unwrap_or_default()
}

fn bus_path(roots: &Roots, sysfs_path: &Path) -> String {
    let Ok(target) = fs::canonicalize(sysfs_path.join("device")) else {
        return String::new();
    };
    let root = fs::canonicalize(&roots.block).unwrap_or_else(|_error| roots.block.clone());

    let target = target.to_string_lossy();
    let root = root.to_string_lossy();

    let mount = root.rsplit_once('/').map_or("", |(parent, _unused)| parent);
    let relative = target
        .strip_prefix(root.as_ref())
        .or_else(|| target.strip_prefix(mount))
        .unwrap_or(target.as_ref());
    let relative = relative
        .strip_prefix("/devices/")
        .or_else(|| relative.strip_prefix("devices/"))
        .unwrap_or(relative);

    format!("/{}", relative.trim_start_matches('/'))
}

fn is_physical_disk(name: &str) -> bool {
    !name.starts_with("loop")
        && !name.starts_with("dm-")
        && !name.starts_with("ram")
        && !name.starts_with("sr")
}
