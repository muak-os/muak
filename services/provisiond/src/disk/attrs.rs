//! Low-level sysfs attribute readers, mount table parsing, and filesystem probes.

use std::io::{Read as _, Seek as _};
use std::path::Path;

use ::disk::role::Role;
use anyhow::Result;
use tokio::fs;

use super::constants::SECTOR_SIZE;
use super::types::PartitionInfo;

/// Path of the kernel mount table.
pub(crate) const MOUNTS_PATH: &str = "/proc/mounts";

/// A device and the mount point where it is mounted.
pub(crate) struct Mount {
    pub device: String,
    pub mount_point: String,
}

/// Reads the partition attributes of `part_name` on `disk_name` below `sysfs_root`.
pub(crate) async fn read_partition_info(
    sysfs_root: &Path,
    disk_name: &str,
    part_name: &str,
    mounts: &[Mount],
) -> Result<PartitionInfo> {
    let sysfs_path = sysfs_root.join(disk_name).join(part_name);

    let number = u32::try_from(read_sysfs_u64(&sysfs_path.join("partition")).await?).unwrap_or(0);
    let start_sector = read_sysfs_u64(&sysfs_path.join("start")).await?;
    let size_sectors = read_sysfs_u64(&sysfs_path.join("size")).await?;
    let size_bytes = size_sectors.saturating_mul(SECTOR_SIZE);

    let path = format!("/dev/{part_name}");
    let fstype = detect_filesystem(&path);
    let gpt_name = read_partname(&sysfs_path).await;
    let mount_point = find_mount_point(mounts, &path).to_owned();
    let in_use = is_in_use(&gpt_name, &mount_point);

    Ok(PartitionInfo {
        number,
        start_sector,
        size_bytes,
        name: part_name.to_owned(),
        path,
        fstype,
        gpt_name,
        mount_point,
        in_use,
    })
}

/// Reads the mount table at `path`.
pub(crate) fn read_mounts(path: &Path) -> Vec<Mount> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };

    parse_mounts(&content)
}

/// Reads the GPT partition name from the uevent of a partition.
pub(crate) async fn read_partname(sysfs_path: &Path) -> String {
    let Ok(content) = fs::read_to_string(sysfs_path.join("uevent")).await else {
        return String::new();
    };

    parse_partname(&content)
}

/// Returns true when `gpt_name` marks the Muak state partition.
pub(crate) fn is_state(gpt_name: &str) -> bool {
    gpt_name == Role::State.gpt_name()
}

/// Returns true when `gpt_name` marks the Muak ephemeral partition.
pub(crate) fn is_ephemeral(gpt_name: &str) -> bool {
    gpt_name == Role::Ephemeral.gpt_name()
}

/// Returns true when `gpt_name` marks the Muak data partition.
pub(crate) fn is_data(gpt_name: &str) -> bool {
    gpt_name == Role::Data.gpt_name()
}

/// Returns true when a partition is mounted or carries a Muak role name.
pub(crate) fn is_in_use(gpt_name: &str, mount_point: &str) -> bool {
    !mount_point.is_empty() || is_state(gpt_name) || is_ephemeral(gpt_name) || is_data(gpt_name)
}

/// Reads a trimmed u64 value from a sysfs attribute file.
pub(crate) async fn read_sysfs_u64(path: &Path) -> Result<u64> {
    let content = fs::read_to_string(path).await?;

    Ok(content.trim().parse()?)
}

/// Reads a trimmed string value from a sysfs attribute file.
pub(crate) async fn read_sysfs_string(path: &Path) -> Result<String> {
    let content = fs::read_to_string(path).await?;

    Ok(content.trim().to_owned())
}

/// Reads the file name of a symlink target, keeping the link unresolved.
pub(crate) fn read_link_name(path: &Path) -> Result<String> {
    let target = std::fs::read_link(path)?;
    let name = match target.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => String::new(),
    };

    Ok(name)
}

fn find_mount_point<'a>(mounts: &'a [Mount], device: &str) -> &'a str {
    mounts
        .iter()
        .find(|mount| is_same_device(&mount.device, device))
        .map_or("", |mount| mount.mount_point.as_str())
}

fn is_same_device(mounted: &str, device: &str) -> bool {
    if mounted == device {
        return true;
    }

    match (
        std::fs::canonicalize(mounted),
        std::fs::canonicalize(device),
    ) {
        (Ok(mounted), Ok(device)) => mounted == device,
        _ => false,
    }
}

fn parse_mounts(content: &str) -> Vec<Mount> {
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

fn parse_partname(content: &str) -> String {
    content
        .lines()
        .find_map(|line| line.trim().strip_prefix("PARTNAME="))
        .map_or_else(String::new, str::to_owned)
}

fn detect_filesystem(device_path: &str) -> String {
    use std::fs::File;

    let Ok(mut file) = File::open(device_path) else {
        return String::new();
    };

    let mut ext4_magic = [0_u8; 2];
    if file.seek(std::io::SeekFrom::Start(0x438)).is_ok()
        && file.read_exact(&mut ext4_magic).is_ok()
        && ext4_magic == [0x53, 0xEF]
    {
        return "ext4".to_owned();
    }

    if let Some(fstype) = detect_fat_filesystem(&mut file) {
        return fstype;
    }

    let mut btrfs_magic = [0_u8; 8];
    if file.seek(std::io::SeekFrom::Start(0x10040)).is_ok()
        && file.read_exact(&mut btrfs_magic).is_ok()
        && btrfs_magic == *b"_BHRfS_M"
    {
        return "btrfs".to_owned();
    }

    String::new()
}

fn detect_fat_filesystem(file: &mut std::fs::File) -> Option<String> {
    if file.seek(std::io::SeekFrom::Start(0)).is_err() {
        return None;
    }

    let mut boot_sector = [0_u8; 512];
    if file.read_exact(&mut boot_sector).is_err() {
        return None;
    }

    let fat32_sig = &boot_sector[82..90];
    if fat32_sig.starts_with(b"FAT32   ") {
        return Some("vfat".to_owned());
    }

    let fat16_sig = &boot_sector[54..62];
    if fat16_sig.starts_with(b"FAT16   ") || fat16_sig.starts_with(b"FAT12   ") {
        return Some("vfat".to_owned());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_partname_reads_the_gpt_name() {
        // ARRANGE
        let uevent = format!(
            "DEVNAME=sda1\nDEVTYPE=partition\nPARTNAME={}\n",
            Role::State.gpt_name()
        );

        // ACT
        let name = parse_partname(&uevent);

        // ASSERT
        assert_eq!(name, Role::State.gpt_name());
    }

    #[test]
    fn parse_partname_defaults_to_empty_without_a_name() {
        // ARRANGE
        let uevent = "DEVNAME=sda1\nDEVTYPE=partition\n";

        // ACT
        let name = parse_partname(uevent);

        // ASSERT
        assert_eq!(name, "");
    }

    #[test]
    fn parse_mounts_reads_device_and_mount_point() {
        // ARRANGE
        let table = "/dev/sda1 /mnt/data ext4 rw,relatime 0 0\n";

        // ACT
        let mounts = parse_mounts(table);

        // ASSERT
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].device, "/dev/sda1");
        assert_eq!(mounts[0].mount_point, "/mnt/data");
    }

    #[test]
    fn find_mount_point_matches_the_device_exactly() {
        // ARRANGE
        let mounts = parse_mounts("/dev/sda10 /ten ext4 rw 0 0\n/dev/sda1 /one ext4 rw 0 0\n");

        // ACT
        let found = find_mount_point(&mounts, "/dev/sda1");

        // ASSERT
        assert_eq!(found, "/one");
    }

    #[test]
    fn find_mount_point_follows_device_symlinks() {
        // ARRANGE
        let root = tempfile::TempDir::new().expect("temp dir");
        let device = root.path().join("sda1");
        std::fs::write(&device, "").expect("device node");
        let link = root.path().join("by-label-DATA");
        std::os::unix::fs::symlink(&device, &link).expect("device link");
        let mounts = vec![Mount {
            device: link.to_string_lossy().into_owned(),
            mount_point: "/mnt/data".to_owned(),
        }];
        let device = device.to_string_lossy().into_owned();

        // ACT
        let found = find_mount_point(&mounts, &device);

        // ASSERT
        assert_eq!(found, "/mnt/data");
    }

    #[test]
    fn is_in_use_when_mounted_or_named_for_a_muak_role() {
        // ARRANGE
        let cases = [
            ("", "/mnt", true),
            (Role::State.gpt_name(), "", true),
            (Role::Ephemeral.gpt_name(), "", true),
            (Role::Data.gpt_name(), "", true),
            (Role::Esp.gpt_name(), "", false),
            ("", "", false),
        ];

        for (gpt_name, mount_point, expected) in cases {
            // ACT
            let result = is_in_use(gpt_name, mount_point);

            // ASSERT
            assert_eq!(result, expected, "{gpt_name} at {mount_point}");
        }
    }
}
