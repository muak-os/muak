//! Conversion of the disk inventory into its protobuf representation.

use super::proto::provision::{DiskInfo, PartitionInfo};
use crate::disk;

/// Converts a discovered disk into its protobuf representation.
pub(crate) fn to_disk_info(disk: disk::DiskInfo) -> DiskInfo {
    DiskInfo {
        name: disk.name,
        path: disk.path,
        size_bytes: disk.size_bytes,
        model: disk.model,
        removable: disk.removable,
        read_only: disk.read_only,
        serial: disk.serial,
        wwid: disk.wwid,
        bus_path: disk.bus_path,
        subsystem: disk.subsystem,
        rotational: disk.rotational,
        cdrom: disk.cdrom,
        symlinks: disk.symlinks,
        is_system: disk.is_system,
        is_data: disk.is_data,
        partitions: disk.partitions.into_iter().map(to_partition_info).collect(),
    }
}

/// Converts a discovered partition into its protobuf representation.
pub(crate) fn to_partition_info(partition: disk::PartitionInfo) -> PartitionInfo {
    PartitionInfo {
        number: partition.number,
        start_sector: partition.start_sector,
        size_bytes: partition.size_bytes,
        name: partition.name,
        path: partition.path,
        fstype: partition.fstype,
        gpt_name: partition.gpt_name,
        mount_point: partition.mount_point,
        in_use: partition.in_use,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_disk_info_maps_the_inventory_fields() {
        // ARRANGE
        let disk = disk::DiskInfo {
            name: "sda".to_owned(),
            path: "/dev/sda".to_owned(),
            size_bytes: 4096,
            model: "Disk".to_owned(),
            serial: "SN123".to_owned(),
            wwid: "naa.5000".to_owned(),
            bus_path: "/pci0000:00/sda".to_owned(),
            subsystem: "scsi".to_owned(),
            rotational: true,
            cdrom: false,
            removable: false,
            read_only: true,
            symlinks: vec!["/dev/disk/by-id/sda".to_owned()],
            is_system: true,
            is_data: true,
            partitions: vec![disk::PartitionInfo {
                number: 1,
                start_sector: 2048,
                size_bytes: 1024,
                name: "sda1".to_owned(),
                path: "/dev/sda1".to_owned(),
                fstype: "ext4".to_owned(),
                gpt_name: "STATE".to_owned(),
                mount_point: "/mnt".to_owned(),
                in_use: true,
            }],
        };

        // ACT
        let proto = to_disk_info(disk);

        // ASSERT
        assert_eq!(proto.serial, "SN123");
        assert_eq!(proto.wwid, "naa.5000");
        assert_eq!(proto.bus_path, "/pci0000:00/sda");
        assert_eq!(proto.subsystem, "scsi");
        assert!(proto.rotational);
        assert!(!proto.cdrom);
        assert_eq!(proto.symlinks, ["/dev/disk/by-id/sda"]);
        assert!(proto.is_system);
        assert!(proto.is_data);

        let partition = &proto.partitions[0];
        assert_eq!(partition.gpt_name, "STATE");
        assert_eq!(partition.mount_point, "/mnt");
        assert!(partition.in_use);
    }
}
