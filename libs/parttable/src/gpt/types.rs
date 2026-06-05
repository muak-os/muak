//! Types related to GPT partition tables.

/// The standard 1 MiB partition alignment in 512-byte sectors.
pub const ALIGN_1_MIB_SECTORS: u64 = 2048;

/// Linux filesystem partition type GUID (0FC63DAF-8483-4772-8E79-3D69D8477DE4).
pub const LINUX_FS_GUID: [u8; 16] = [
    0xaf, 0x3d, 0xc6, 0x0f, 0x83, 0x84, 0x72, 0x47, 0x8e, 0x79, 0x3d, 0x69, 0xd8, 0x47, 0x7d, 0xe4,
];

/// The EFI System Partition type GUID (C12A7328-F81F-11D2-BA4B-00A0C93EC93B).
pub const EFI_GUID: [u8; 16] = [
    0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b,
];

/// A GPT partition entry in a crate-local representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    /// GPT partition type GUID.
    pub type_guid: [u8; 16],
    /// Unique partition GUID.
    pub unique_guid: [u8; 16],
    /// First LBA of the partition.
    pub starting_lba: u64,
    /// Last LBA of the partition (inclusive).
    pub ending_lba: u64,
    /// Partition attributes bitfield.
    pub attributes: u64,
    /// Partition name.
    pub name: String,
}

/// Selects how a partition slot should be chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// Automatically select the first available slot.
    Auto,
    /// Use the exact slot number given.
    Exact(u32),
}

/// Selects how a partition start LBA should be chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// Start at the first usable LBA.
    FirstUsable,
    /// Start after the last used partition.
    AfterLastUsed,
    /// Start at or after the given LBA.
    AtOrAfter(u64),
    /// Start after the partition with the given number.
    AfterPartition(u32),
}

/// Selects how a partition size should be chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    /// Size in bytes.
    Bytes(u64),
    /// Size in LBAs (512-byte sectors).
    Lbas(u64),
    /// Fill to the last usable LBA.
    FillToLastUsable,
}

/// Describes one checked placement request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacementRequest {
    /// How the partition slot is chosen.
    pub slot: Slot,
    /// How the partition start LBA is chosen.
    pub start: Start,
    /// How the partition size is chosen.
    pub size: Size,
    /// Alignment boundary in LBAs.
    pub alignment_lba: u64,
    /// GPT partition type GUID.
    pub type_guid: [u8; 16],
    /// Unique partition GUID.
    pub unique_guid: [u8; 16],
    /// Partition attributes bitfield.
    pub attributes: u64,
    /// Partition name.
    pub name: String,
}

/// Returns the resolved partition placement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    /// Partition number (1-based index).
    pub number: u32,
    /// The resolved partition entry.
    pub partition: Partition,
}
