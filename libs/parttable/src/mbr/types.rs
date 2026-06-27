//! Types and constants related to the Master Boot Record (MBR) partitioning scheme.

/// The byte offset of the first MBR partition entry.
pub const MBR_PARTITION_ENTRY_OFFSET: u64 = 446;

/// The canonical MBR boot signature.
pub const MBR_BOOT_SIGNATURE: [u8; 2] = [0x55, 0xAA];

/// The EFI System Partition MBR type.
pub const MBR_EFI_SYSTEM_TYPE: u8 = 0xEF;

/// The protective partition type for GPT disks.
pub const MBR_PROTECTIVE_GPT_TYPE: u8 = 0xEE;

/// A single MBR partition entry in LBA form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MbrPartitionEntry {
    /// Whether the partition is bootable.
    pub bootable: bool,
    /// MBR partition type byte.
    pub partition_type: u8,
    /// Starting LBA of the partition.
    pub starting_lba: u32,
    /// Size of the partition in LBAs.
    pub size_lba: u32,
}

pub(crate) const MBR_BYTES: usize = 512;
