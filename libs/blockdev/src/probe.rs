//! Filesystem signature probing on a partition device.

use std::fs::File;
use std::path::Path;

use crate::io;

const EXT4_MAGIC_OFFSET: u64 = 0x438;
const EXT4_MAGIC: [u8; 2] = [0x53, 0xEF];

const BTRFS_MAGIC_OFFSET: u64 = 0x10040;
const BTRFS_MAGIC: [u8; 8] = *b"_BHRfS_M";

const FAT32_SIGNATURE: core::ops::Range<usize> = 82..90;
const FAT16_SIGNATURE: core::ops::Range<usize> = 54..62;

/// Detects the filesystem on `device` from its magic bytes.
pub(crate) fn filesystem(device: &Path) -> Option<&'static str> {
    let file = File::open(device).ok()?;

    if has_magic(&file, EXT4_MAGIC_OFFSET, &EXT4_MAGIC) {
        return Some("ext4");
    }

    if let Some(filesystem) = fat(&file) {
        return Some(filesystem);
    }

    if has_magic(&file, BTRFS_MAGIC_OFFSET, &BTRFS_MAGIC) {
        return Some("btrfs");
    }

    None
}

fn has_magic(file: &File, offset: u64, expected: &[u8]) -> bool {
    let mut magic = [0_u8; BTRFS_MAGIC.len()];
    let Some(slot) = magic.get_mut(..expected.len()) else {
        return false;
    };

    io::read_at(file, offset, slot).is_ok() && slot == expected
}

fn fat(file: &File) -> Option<&'static str> {
    let mut boot_sector = [0_u8; 512];
    if io::read_at(file, 0, &mut boot_sector).is_err() {
        return None;
    }

    let fat32 = boot_sector.get(FAT32_SIGNATURE).unwrap_or_default();
    if fat32.starts_with(b"FAT32   ") {
        return Some("vfat");
    }

    let fat16 = boot_sector.get(FAT16_SIGNATURE).unwrap_or_default();
    if fat16.starts_with(b"FAT16   ") || fat16.starts_with(b"FAT12   ") {
        return Some("vfat");
    }

    None
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::FileExt as _;

    use super::*;

    const PROBE_FILE_SIZE: u64 = 0x2_0000;

    fn file_with(bytes: &[u8], offset: u64) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().unwrap();
        file.as_file().set_len(PROBE_FILE_SIZE).unwrap();
        file.as_file().write_all_at(bytes, offset).unwrap();
        file
    }

    #[test]
    fn filesystem_detects_a_btrfs_magic() {
        // ARRANGE
        let file = file_with(&BTRFS_MAGIC, BTRFS_MAGIC_OFFSET);

        // ACT
        let filesystem = filesystem(file.path());

        // ASSERT
        assert_eq!(filesystem, Some("btrfs"));
    }

    #[test]
    fn filesystem_detects_a_fat32_signature() {
        // ARRANGE
        let offset = u64::try_from(FAT32_SIGNATURE.start).unwrap();
        let file = file_with(b"FAT32   ", offset);

        // ACT
        let filesystem = filesystem(file.path());

        // ASSERT
        assert_eq!(filesystem, Some("vfat"));
    }
}
