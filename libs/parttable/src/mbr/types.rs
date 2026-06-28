//! Types and constants related to the Master Boot Record (MBR) partitioning scheme.

/// The byte offset of the first MBR partition entry.
pub const MBR_PARTITION_ENTRY_OFFSET: u64 = 446;

/// The canonical MBR boot signature.
pub const MBR_BOOT_SIGNATURE: [u8; 2] = [0x55, 0xAA];

/// The EFI System Partition MBR type.
pub const MBR_EFI_SYSTEM_TYPE: u8 = 0xEF;

/// The protective partition type for GPT disks.
pub const MBR_PROTECTIVE_GPT_TYPE: u8 = 0xEE;

/// Size of a complete MBR sector in bytes.
pub const MBR_BYTES: usize = 512;

/// Size of a single MBR partition entry in bytes.
pub const MBR_ENTRY_SIZE: usize = 16;

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

impl MbrPartitionEntry {
    /// Serializes this entry into a 16-byte MBR partition entry.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; MBR_ENTRY_SIZE] {
        let mut entry = [0_u8; MBR_ENTRY_SIZE];
        entry[0] = u8::from(self.bootable);
        entry[1..4].copy_from_slice(&[0x00, 0x02, 0x00]);
        entry[4] = self.partition_type;
        entry[5..8].copy_from_slice(&[0xFF, 0xFF, 0xFF]);
        entry[8..12].copy_from_slice(&self.starting_lba.to_le_bytes());
        entry[12..16].copy_from_slice(&self.size_lba.to_le_bytes());

        entry
    }
}
