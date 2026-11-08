//! Kernel mount and swap tables.

use std::fs;
use std::path::Path;

/// A device and the mount point where it is mounted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    /// Device or path the kernel reports as mounted.
    pub device: String,
    /// Mount point of the device.
    pub mount_point: String,
}

/// A swap area reported by the kernel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Swap {
    /// Device or file backing the swap area.
    pub device: String,
}

/// Reads the mount table at `path`, returning nothing when it cannot be read.
#[must_use]
pub fn read(path: &Path) -> Vec<Mount> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };

    parse(&content)
}

/// Returns the mount point of `device`, empty when it is not mounted.
#[must_use]
pub fn of<'a>(mounts: &'a [Mount], device: &str) -> &'a str {
    mounts
        .iter()
        .find(|mount| is_same_device(&mount.device, device))
        .map_or("", |mount| mount.mount_point.as_str())
}

fn parse(content: &str) -> Vec<Mount> {
    content
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let device = fields.next()?;
            let mount_point = fields.next()?;

            Some(Mount {
                device: device.to_owned(),
                mount_point: mount_point.to_owned(),
            })
        })
        .collect()
}

fn is_same_device(mounted: &str, device: &str) -> bool {
    if mounted == device {
        return true;
    }

    match (fs::canonicalize(mounted), fs::canonicalize(device)) {
        (Ok(mounted), Ok(device)) => mounted == device,
        _ => false,
    }
}
