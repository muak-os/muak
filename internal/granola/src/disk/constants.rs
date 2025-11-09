pub const SECTOR_SIZE: u64 = 512;
pub const MB: u64 = 1024 * 1024;
pub const GB: u64 = 1024 * MB;

pub const EFI_SIZE: u64 = 512 * MB; // 512 MB for EFI
pub const STATE_SIZE: u64 = 1 * GB; // 1 GB for STATE
pub const MIN_DISK_SIZE: u64 = 2 * GB; // Minimum 2 GB total

// GPT Partition Type GUIDs
pub const EFI_GUID: [u8; 16] = [
    0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b,
]; // C12A7328-F81F-11D2-BA4B-00A0C93EC93B

pub const LINUX_FS_GUID: [u8; 16] = [
    0xaf, 0x3d, 0xc6, 0x0f, 0x83, 0x84, 0x72, 0x47, 0x8e, 0x79, 0x3d, 0x69, 0xd8, 0x47, 0x7d, 0xe4,
]; // 0FC63DAF-8483-4772-8E79-3D69D8477DE4
