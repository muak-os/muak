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
    pub bootable: bool,
    pub partition_type: u8,
    pub starting_lba: u32,
    pub size_lba: u32,
}

pub(crate) const MBR_BYTES: usize = 512;
pub(crate) const MBR_ENTRY_BYTES: usize = 16;
pub(crate) const MBR_MAX_SLOTS: u8 = 4;
pub(crate) const MBR_CHS_LBA_PLACEHOLDER: [u8; 3] = [0xFE, 0xFF, 0xFF];
pub(crate) const MBR_STARTING_LBA_OFFSET: usize = 8;
pub(crate) const MBR_SIZE_LBA_OFFSET: usize = 12;
pub(crate) const MBR_PARTITION_TYPE_OFFSET: usize = 4;
pub(crate) const MBR_PARTITION_STARTING_LBA: u32 = 1;
pub(crate) const MBR_BOOT_SIGNATURE_OFFSET: u64 = 510;
