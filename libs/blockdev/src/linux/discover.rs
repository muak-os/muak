//! Partition lookup by GPT name and resolution of the disk owning a partition.

use std::fs;
use std::path::Path;

use super::{Roots, attrs};

/// Finds the partition device paths carrying `partname` below injectable roots.
#[must_use]
pub(crate) fn find_by_partname_in(roots: &Roots, partname: &str) -> Vec<String> {
    let Ok(entries) = fs::read_dir(&roots.class_block) else {
        return Vec::new();
    };

    let mut devices = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let path = entry.path();

        if !path.join("partition").exists() {
            continue;
        }

        let Ok(content) = fs::read_to_string(path.join("uevent")) else {
            continue;
        };
        if attrs::partname(&content) != partname {
            continue;
        }

        let device = roots.dev.join(name.as_ref());
        if device.exists() {
            devices.push(device.to_string_lossy().into_owned());
        }
    }

    devices.sort_unstable();

    devices
}

/// Returns the name of the disk carrying `partition_device` below injectable roots.
#[must_use]
pub(crate) fn parent_disk_in(roots: &Roots, partition_device: &str) -> Option<String> {
    let name = Path::new(partition_device).file_name()?;
    let resolved = fs::canonicalize(roots.class_block.join(name)).ok()?;

    resolved.parent()?.file_name()?.to_str().map(str::to_owned)
}
