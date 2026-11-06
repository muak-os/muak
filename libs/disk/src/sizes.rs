//! Frozen product disk geometry and type identifiers.

/// GPT type GUID of the EFI System Partition (wire byte order).
pub const ESP_TYPE_GUID: [u8; 16] = [
    0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b,
];

/// Frozen size of the EFI System Partition.
pub const EFI_SIZE: u64 = 512 * 1024 * 1024;

/// Frozen size of the STATE partition.
pub const STATE_SIZE: u64 = 1024 * 1024 * 1024;

/// Frozen size of the EPHEMERAL partition.
pub const EPHEMERAL_SIZE: u64 = 10 * 1024 * 1024 * 1024;
