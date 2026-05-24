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
    pub type_guid: [u8; 16],
    pub unique_guid: [u8; 16],
    pub starting_lba: u64,
    pub ending_lba: u64,
    pub attributes: u64,
    pub name: String,
}

/// Selects how a partition slot should be chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Auto,
    Exact(u32),
}

/// Selects how a partition start LBA should be chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    FirstUsable,
    AfterLastUsed,
    AtOrAfter(u64),
    AfterPartition(u32),
}

/// Selects how a partition size should be chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Bytes(u64),
    Lbas(u64),
    FillToLastUsable,
}

/// Describes one checked placement request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacementRequest {
    pub slot: Slot,
    pub start: Start,
    pub size: Size,
    pub alignment_lba: u64,
    pub type_guid: [u8; 16],
    pub unique_guid: [u8; 16],
    pub attributes: u64,
    pub name: String,
}

/// Returns the resolved partition placement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub number: u32,
    pub partition: Partition,
}
