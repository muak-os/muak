//! Sysfs-based disk and partition discovery utilities.

use std::io::Seek as _;
use std::os::unix::fs::FileTypeExt as _;
use std::path::Path;

use anyhow::{Result, bail};
use tokio::fs;

use super::attrs::{
    MOUNTS_PATH, Mount, is_data, is_state, read_link_name, read_mounts, read_partition_info,
    read_sysfs_string, read_sysfs_u64,
};
use super::constants::{GB, MB, MIN_DISK_SIZE, SECTOR_SIZE};
use super::types::{DiskInfo, PartitionInfo};

/// Path of the block device tree, one directory per disk.
pub(crate) const SYSFS_BLOCK_ROOT: &str = "/sys/block";

/// Path of the flat block device class tree, disks and partitions side by side.
pub(crate) const SYSFS_CLASS_BLOCK_ROOT: &str = "/sys/class/block";

/// Validates that a disk meets minimum size requirements.
pub fn validate_disk_size(disk: &str) -> Result<()> {
    let mut file = std::fs::File::open(disk)?;
    let disk_size = file.seek(std::io::SeekFrom::End(0))?;

    if disk_size < MIN_DISK_SIZE {
        bail!(
            "Disk '{}' is too small ({} MB). Minimum required: {} MB",
            disk,
            disk_size.checked_div(MB).unwrap_or(0),
            MIN_DISK_SIZE.checked_div(MB).unwrap_or(0)
        );
    }

    kmsg::info!(
        "Disk size: {} GB ({} MB)",
        disk_size.checked_div(GB).unwrap_or(0),
        disk_size.checked_div(MB).unwrap_or(0)
    );

    Ok(())
}

/// Validates that the specified path is a block device.
pub fn validate_block_device(disk: &str) -> Result<()> {
    let metadata = std::fs::metadata(disk)?;

    if !metadata.file_type().is_block_device() {
        bail!("'{disk}' is not a block device. Please specify a disk (e.g., /dev/sda, /dev/vda)");
    }

    if disk.chars().last().is_some_and(char::is_numeric)
        && (disk.contains("sd") || disk.contains("vd") || disk.contains("hd"))
    {
        kmsg::warn!(
            "'{}' appears to be a partition. You should install to a whole disk (e.g., /dev/sda, not /dev/sda1)",
            disk
        );
    }

    Ok(())
}

/// Lists the physical disks reported by the running kernel through sysfs.
pub async fn list_disks() -> Result<Vec<DiskInfo>> {
    let mounts = read_mounts(Path::new(MOUNTS_PATH));

    list_disks_in(Path::new(SYSFS_BLOCK_ROOT), &mounts).await
}

async fn list_disks_in(sysfs_root: &Path, mounts: &[Mount]) -> Result<Vec<DiskInfo>> {
    if !sysfs_root.exists() {
        bail!(
            "{} does not exist - sysfs not mounted?",
            sysfs_root.display()
        );
    }

    let mut disks = Vec::new();
    let mut entries = fs::read_dir(sysfs_root).await?;

    while let Ok(Some(entry)) = entries.next_entry().await {
        if let Some(disk) = try_read_disk(sysfs_root, entry, mounts).await {
            disks.push(disk);
        }
    }

    disks.sort_by(|left, right| left.name.cmp(&right.name));

    Ok(disks)
}

async fn try_read_disk(
    sysfs_root: &Path,
    entry: fs::DirEntry,
    mounts: &[Mount],
) -> Option<DiskInfo> {
    let name = entry.file_name();
    let name_str = name.to_string_lossy();

    if !is_physical_disk(&name_str) {
        return None;
    }

    match read_disk_info_in(sysfs_root, &name_str, mounts).await {
        Ok(disk_info) if disk_info.size_bytes > 0 => Some(disk_info),
        Ok(_) => None,
        Err(e) => {
            kmsg::error!("Failed to read disk {}: {}", name_str, e);
            None
        }
    }
}

async fn read_disk_info_in(sysfs_root: &Path, name: &str, mounts: &[Mount]) -> Result<DiskInfo> {
    let sysfs_path = sysfs_root.join(name);

    let size_sectors = read_sysfs_u64(&sysfs_path.join("size")).await?;
    let size_bytes = size_sectors.saturating_mul(SECTOR_SIZE);
    let removable = read_sysfs_u64(&sysfs_path.join("removable")).await? != 0;
    let read_only = read_sysfs_u64(&sysfs_path.join("ro")).await? != 0;

    let model = read_sysfs_string(&sysfs_path.join("device/model"))
        .await
        .unwrap_or_else(|_| name.to_owned());

    let serial = read_fact(&sysfs_path, "device/serial", "serial").await;
    let wwid = read_fact(&sysfs_path, "wwid", "device/wwid").await;
    let rotational = read_sysfs_u64(&sysfs_path.join("queue/rotational"))
        .await
        .unwrap_or(0)
        != 0;
    let bus_path = resolve_bus_path(sysfs_root, &sysfs_path);
    let subsystem = read_link_name(&sysfs_path.join("device/subsystem")).unwrap_or_default();

    let path = format!("/dev/{name}");

    let partitions = discover_partitions(sysfs_root, &sysfs_path, name, mounts).await;
    let is_system = partitions
        .iter()
        .any(|partition| is_state(&partition.gpt_name));
    let is_data = partitions
        .iter()
        .any(|partition| is_data(&partition.gpt_name));

    Ok(DiskInfo {
        name: name.to_owned(),
        path,
        size_bytes,
        model,
        serial,
        wwid,
        bus_path,
        subsystem,
        rotational,
        cdrom: false,
        removable,
        read_only,
        symlinks: Vec::new(),
        is_system,
        is_data,
        partitions,
    })
}

async fn discover_partitions(
    sysfs_root: &Path,
    sysfs_path: &Path,
    disk_name: &str,
    mounts: &[Mount],
) -> Vec<PartitionInfo> {
    let Ok(mut entries) = fs::read_dir(sysfs_path).await else {
        return Vec::new();
    };

    let mut partitions = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        if let Some(part) = try_read_partition(sysfs_root, entry, disk_name, mounts).await {
            partitions.push(part);
        }
    }

    partitions.sort_by_key(|part| part.number);

    partitions
}

async fn try_read_partition(
    sysfs_root: &Path,
    entry: fs::DirEntry,
    disk_name: &str,
    mounts: &[Mount],
) -> Option<PartitionInfo> {
    let part_name = entry.file_name();
    let part_name_str = part_name.to_string_lossy();

    if !part_name_str.starts_with(disk_name) || part_name_str == disk_name {
        return None;
    }

    if !entry.path().join("partition").exists() {
        return None;
    }

    read_partition_info(sysfs_root, disk_name, &part_name_str, mounts)
        .await
        .ok()
}

async fn read_fact(sysfs_path: &Path, primary: &str, fallback: &str) -> String {
    if let Ok(value) = read_sysfs_string(&sysfs_path.join(primary)).await {
        return value;
    }

    read_sysfs_string(&sysfs_path.join(fallback))
        .await
        .unwrap_or_default()
}

fn resolve_bus_path(sysfs_root: &Path, sysfs_path: &Path) -> String {
    let Ok(target) = std::fs::canonicalize(sysfs_path.join("device")) else {
        return String::new();
    };
    let root = std::fs::canonicalize(sysfs_root).unwrap_or_else(|_| sysfs_root.to_path_buf());

    let target = target.to_string_lossy();
    let root = root.to_string_lossy();

    let mount = root.rsplit_once('/').map_or("", |(parent, _)| parent);
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
        && !name.starts_with("sr") // CD-ROM
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::Role;

    #[test]
    fn is_physical_disk_rejects_loop_devices() {
        // ARRANGE
        let names = ["loop0", "loop1", "loop99"];

        for name in &names {
            // ACT
            let result = is_physical_disk(name);

            // ASSERT
            assert!(!result, "{name} should not be a physical disk");
        }
    }

    #[test]
    fn is_physical_disk_rejects_device_mapper() {
        // ARRANGE
        let names = ["dm-0", "dm-1", "dm-99"];

        for name in &names {
            // ACT
            let result = is_physical_disk(name);

            // ASSERT
            assert!(!result, "{name} should not be a physical disk");
        }
    }

    #[test]
    fn is_physical_disk_rejects_ram_devices() {
        // ARRANGE
        let names = ["ram0", "ram1"];

        for name in &names {
            // ACT
            let result = is_physical_disk(name);

            // ASSERT
            assert!(!result, "{name} should not be a physical disk");
        }
    }

    #[test]
    fn is_physical_disk_rejects_cdrom_devices() {
        // ARRANGE
        let names = ["sr0", "sr1"];

        for name in &names {
            // ACT
            let result = is_physical_disk(name);

            // ASSERT
            assert!(!result, "{name} should not be a physical disk");
        }
    }

    #[test]
    fn is_physical_disk_accepts_real_disk_names() {
        // ARRANGE
        let names = ["sda", "sdb", "sdc", "vda", "nvme0n1", "mmcblk0"];

        for name in &names {
            // ACT
            let result = is_physical_disk(name);

            // ASSERT
            assert!(result, "{name} should be a physical disk");
        }
    }

    #[test]
    fn read_disk_info_in_reads_stable_facts() {
        // ARRANGE
        let root = tempfile::TempDir::new().expect("temp dir");
        write_disk(root.path(), "nvme0n1");
        std::fs::write(root.path().join("nvme0n1/device/serial"), "SN123\n").expect("serial");
        std::fs::write(root.path().join("nvme0n1/wwid"), "naa.5000\n").expect("wwid");
        std::fs::write(root.path().join("nvme0n1/queue/rotational"), "0\n").expect("rotational");
        link_subsystem(root.path(), "nvme0n1", "nvme");

        // ACT
        let disk = run(read_disk_info_in(root.path(), "nvme0n1", &[])).expect("disk info");

        // ASSERT
        assert_eq!(disk.serial, "SN123");
        assert_eq!(disk.wwid, "naa.5000");
        assert_eq!(disk.subsystem, "nvme");
        assert!(!disk.rotational);
        assert_eq!(disk.bus_path, "/nvme0n1/device");
    }

    #[test]
    fn read_disk_info_in_falls_back_to_disk_attributes() {
        // ARRANGE
        let root = tempfile::TempDir::new().expect("temp dir");
        write_disk(root.path(), "sda");
        std::fs::write(root.path().join("sda/serial"), "FALLBACK-SERIAL\n").expect("serial");
        std::fs::write(root.path().join("sda/device/wwid"), "fallback.wwid\n").expect("wwid");
        link_subsystem(root.path(), "sda", "scsi");

        // ACT
        let disk = run(read_disk_info_in(root.path(), "sda", &[])).expect("disk info");

        // ASSERT
        assert_eq!(disk.serial, "FALLBACK-SERIAL");
        assert_eq!(disk.wwid, "fallback.wwid");
        assert_eq!(disk.subsystem, "scsi");
    }

    #[test]
    fn read_disk_info_in_defaults_missing_facts() {
        // ARRANGE
        let root = tempfile::TempDir::new().expect("temp dir");
        write_disk(root.path(), "sda");

        // ACT
        let disk = run(read_disk_info_in(root.path(), "sda", &[])).expect("disk info");

        // ASSERT
        assert_eq!(disk.serial, "");
        assert_eq!(disk.wwid, "");
        assert_eq!(disk.subsystem, "");
        assert_eq!(disk.bus_path, "/sda/device");
        assert!(!disk.rotational);
        assert!(!disk.cdrom);
        assert_eq!(disk.symlinks, Vec::<String>::new());
        assert!(!disk.is_system);
        assert!(!disk.is_data);
    }

    #[test]
    fn read_disk_info_in_resolves_bus_path_through_devices_tree() {
        // ARRANGE
        let root = tempfile::TempDir::new().expect("temp dir");
        let block = root.path().join("sys/block");
        write_disk(&block, "sda");
        std::fs::remove_dir_all(block.join("sda/device")).expect("placeholder device dir");
        let device = root.path().join("sys/devices/pci0000:00/ata1/sda");
        std::fs::create_dir_all(&device).expect("device tree");
        std::os::unix::fs::symlink(&device, block.join("sda/device")).expect("device link");

        // ACT
        let disk = run(read_disk_info_in(&block, "sda", &[])).expect("disk info");

        // ASSERT
        assert_eq!(disk.bus_path, "/pci0000:00/ata1/sda");
    }

    #[test]
    fn read_disk_info_in_marks_the_state_partition_as_system() {
        // ARRANGE
        let root = tempfile::TempDir::new().expect("temp dir");
        write_disk(root.path(), "sda");
        write_partition(root.path(), "sda", "sda1", Role::State.gpt_name());

        // ACT
        let disk = run(read_disk_info_in(root.path(), "sda", &[])).expect("disk info");

        // ASSERT
        assert!(disk.is_system);
        assert!(!disk.is_data);
        assert_eq!(disk.partitions[0].gpt_name, Role::State.gpt_name());
        assert!(disk.partitions[0].in_use);
    }

    #[test]
    fn read_disk_info_in_marks_the_data_partition() {
        // ARRANGE
        let root = tempfile::TempDir::new().expect("temp dir");
        write_disk(root.path(), "sda");
        write_partition(root.path(), "sda", "sda1", Role::Data.gpt_name());

        // ACT
        let disk = run(read_disk_info_in(root.path(), "sda", &[])).expect("disk info");

        // ASSERT
        assert!(!disk.is_system);
        assert!(disk.is_data);
        assert!(disk.partitions[0].in_use);
    }

    #[test]
    fn read_disk_info_in_marks_a_shared_disk_as_system_and_data() {
        // ARRANGE
        let root = tempfile::TempDir::new().expect("temp dir");
        write_disk(root.path(), "sda");
        write_partition(root.path(), "sda", "sda1", Role::State.gpt_name());
        write_partition(root.path(), "sda", "sda2", Role::Data.gpt_name());

        // ACT
        let disk = run(read_disk_info_in(root.path(), "sda", &[])).expect("disk info");

        // ASSERT
        assert!(disk.is_system);
        assert!(disk.is_data);
    }

    #[test]
    fn read_disk_info_in_records_mount_points_and_marks_them_in_use() {
        // ARRANGE
        let root = tempfile::TempDir::new().expect("temp dir");
        write_disk(root.path(), "sda");
        write_partition(root.path(), "sda", "sda1", "");
        let mounts = vec![Mount {
            device: "/dev/sda1".to_owned(),
            mount_point: "/mnt/data".to_owned(),
        }];

        // ACT
        let disk = run(read_disk_info_in(root.path(), "sda", &mounts)).expect("disk info");

        // ASSERT
        assert_eq!(disk.partitions[0].mount_point, "/mnt/data");
        assert!(disk.partitions[0].in_use);
    }

    #[test]
    fn list_disks_in_sorts_by_name() {
        // ARRANGE
        let root = tempfile::TempDir::new().expect("temp dir");
        write_disk(root.path(), "sdb");
        write_disk(root.path(), "sda");

        // ACT
        let disks = run(list_disks_in(root.path(), &[])).expect("disks");

        // ASSERT
        let names: Vec<&str> = disks.iter().map(|disk| disk.name.as_str()).collect();
        assert_eq!(names, ["sda", "sdb"]);
    }

    #[test]
    fn list_disks_in_discovers_physical_disks_only() {
        // ARRANGE
        let root = tempfile::TempDir::new().expect("temp dir");
        write_disk(root.path(), "sda");
        write_disk(root.path(), "loop0");

        // ACT
        let disks = run(list_disks_in(root.path(), &[])).expect("disks");

        // ASSERT
        assert_eq!(disks.len(), 1);
        assert_eq!(disks[0].name, "sda");
        assert_eq!(disks[0].path, "/dev/sda");
        assert_eq!(disks[0].size_bytes, 2048 * SECTOR_SIZE);
    }

    fn write_disk(root: &Path, name: &str) {
        let disk = root.join(name);
        std::fs::create_dir_all(disk.join("device")).expect("device dir");
        std::fs::create_dir_all(disk.join("queue")).expect("queue dir");
        std::fs::write(disk.join("size"), "2048\n").expect("size");
        std::fs::write(disk.join("removable"), "0\n").expect("removable");
        std::fs::write(disk.join("ro"), "0\n").expect("ro");
        std::fs::write(disk.join("device/model"), "Fake Disk\n").expect("model");
    }

    fn write_partition(root: &Path, disk: &str, part: &str, gpt_name: &str) {
        let path = root.join(disk).join(part);
        std::fs::create_dir_all(&path).expect("partition dir");
        std::fs::write(path.join("partition"), "1\n").expect("partition");
        std::fs::write(path.join("start"), "2048\n").expect("start");
        std::fs::write(path.join("size"), "1024\n").expect("size");
        std::fs::write(
            path.join("uevent"),
            format!("DEVNAME={part}\nDEVTYPE=partition\nPARTNAME={gpt_name}\n"),
        )
        .expect("uevent");
    }

    fn link_subsystem(root: &Path, name: &str, subsystem: &str) {
        let target = format!("../../../../bus/{subsystem}");
        std::os::unix::fs::symlink(target, root.join(name).join("device/subsystem"))
            .expect("subsystem link");
    }

    fn run<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(future)
    }
}
