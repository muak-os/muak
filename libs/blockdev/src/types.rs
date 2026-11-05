//! Neutral descriptions of a block device and its partitions.

/// A physical disk reported by the kernel.
#[derive(Debug, Clone, Default)]
pub struct Disk {
    /// Kernel device name, for example `sda`.
    pub name: String,
    /// Kernel device path, for example `/dev/sda`.
    pub path: String,
    /// Capacity in bytes.
    pub size_bytes: u64,
    /// Model string reported by the device.
    pub model: String,
    /// Serial string reported by the device.
    pub serial: String,
    /// World wide identifier reported by the device.
    pub wwid: String,
    /// Bus path of the device below `/sys/devices`.
    pub bus_path: String,
    /// Subsystem the device is attached to, for example `scsi`.
    pub subsystem: String,
    /// Whether the device reports rotational media.
    pub rotational: bool,
    /// Whether the device is removable.
    pub removable: bool,
    /// Whether the device is read-only.
    pub read_only: bool,
    /// Partitions the kernel reports on the disk, ordered by number.
    pub partitions: Vec<Partition>,
}

/// A partition of a physical disk.
#[derive(Debug, Clone, Default)]
pub struct Partition {
    /// Partition number in the partition table.
    pub number: u32,
    /// First sector of the partition.
    pub start_sector: u64,
    /// Capacity in bytes.
    pub size_bytes: u64,
    /// Kernel device name, for example `sda1`.
    pub name: String,
    /// Kernel device path, for example `/dev/sda1`.
    pub path: String,
    /// Filesystem detected on the partition, empty when none is recognised.
    pub fstype: String,
    /// GPT partition name, empty when the table carries none.
    pub gpt_name: String,
    /// Mount point the partition is mounted on, empty when unmounted.
    pub mount_point: String,
}
