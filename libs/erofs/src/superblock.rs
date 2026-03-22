//! On-disk superblock serialization.

use crate::BLOCK_SIZE;

pub const EROFS_SUPER_MAGIC_V1: u32 = 0xE0F5_E1E2;
pub const EROFS_SUPER_OFFSET: usize = 1024;
pub const EROFS_FEATURE_COMPAT_SB_CHKSUM: u32 = 0x0000_0001;
pub const EROFS_FEATURE_COMPAT_MTIME: u32 = 0x0000_0002;

/// Parameters for writing the on-disk superblock.
pub struct SuperblockParams {
    pub root_nid: u16,
    pub inos: u64,
    pub epoch: u64,
    pub blocks: u32,
    pub uuid: [u8; 16],
}

/// Serialize the 128-byte on-disk superblock into `buf` at the correct offset.
pub fn write_superblock(buf: &mut [u8], p: &SuperblockParams) {
    let sb = &mut buf[EROFS_SUPER_OFFSET..EROFS_SUPER_OFFSET + 128];

    sb[0x00..0x04].copy_from_slice(&EROFS_SUPER_MAGIC_V1.to_le_bytes());
    sb[0x04..0x08].copy_from_slice(&0u32.to_le_bytes());
    let feature_compat = EROFS_FEATURE_COMPAT_SB_CHKSUM | EROFS_FEATURE_COMPAT_MTIME;
    sb[0x08..0x0C].copy_from_slice(&feature_compat.to_le_bytes());
    sb[0x0C] = 12;
    sb[0x0D] = 0;
    sb[0x0E..0x10].copy_from_slice(&p.root_nid.to_le_bytes());
    sb[0x10..0x18].copy_from_slice(&p.inos.to_le_bytes());
    sb[0x18..0x20].copy_from_slice(&p.epoch.to_le_bytes());
    sb[0x20..0x24].copy_from_slice(&0u32.to_le_bytes());
    sb[0x24..0x28].copy_from_slice(&p.blocks.to_le_bytes());
    sb[0x28..0x2C].copy_from_slice(&0u32.to_le_bytes()); // meta_blkaddr = 0
    sb[0x2C..0x30].copy_from_slice(&0u32.to_le_bytes()); // xattr_blkaddr = 0
    sb[0x30..0x40].copy_from_slice(&p.uuid);
}

/// Compute and write the CRC32-C checksum over the superblock region.
pub fn write_checksum(buf: &mut [u8]) {
    buf[EROFS_SUPER_OFFSET + 0x04..EROFS_SUPER_OFFSET + 0x08].copy_from_slice(&0u32.to_le_bytes());
    let crc = !crc32c::crc32c(&buf[EROFS_SUPER_OFFSET..BLOCK_SIZE as usize]);
    buf[EROFS_SUPER_OFFSET + 0x04..EROFS_SUPER_OFFSET + 0x08].copy_from_slice(&crc.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_params() -> SuperblockParams {
        SuperblockParams {
            root_nid: 0,
            inos: 1,
            epoch: 0,
            blocks: 1,
            uuid: [0; 16],
        }
    }

    #[test]
    fn magic_at_correct_offset() {
        // ARRANGE
        let mut buf = vec![0u8; BLOCK_SIZE as usize];

        // ACT
        write_superblock(&mut buf, &test_params());

        // ASSERT
        let magic = u32::from_le_bytes(
            buf[EROFS_SUPER_OFFSET..EROFS_SUPER_OFFSET + 4]
                .try_into()
                .expect("4 bytes"),
        );
        assert_eq!(magic, EROFS_SUPER_MAGIC_V1);
    }

    #[test]
    fn blkszbits_is_12() {
        // ARRANGE
        let mut buf = vec![0u8; BLOCK_SIZE as usize];

        // ACT
        write_superblock(&mut buf, &test_params());

        // ASSERT
        assert_eq!(buf[EROFS_SUPER_OFFSET + 0x0C], 12);
    }

    #[test]
    fn meta_blkaddr_is_zero() {
        // ARRANGE
        let mut buf = vec![0u8; BLOCK_SIZE as usize];

        // ACT
        write_superblock(&mut buf, &test_params());

        // ASSERT
        let meta = u32::from_le_bytes(
            buf[EROFS_SUPER_OFFSET + 0x28..EROFS_SUPER_OFFSET + 0x2C]
                .try_into()
                .expect("4 bytes"),
        );
        assert_eq!(meta, 0);
    }

    #[test]
    fn checksum_roundtrip() {
        // ARRANGE
        let mut buf = vec![0u8; BLOCK_SIZE as usize];
        write_superblock(&mut buf, &test_params());

        // ACT
        write_checksum(&mut buf);

        // ASSERT
        let stored = u32::from_le_bytes(
            buf[EROFS_SUPER_OFFSET + 4..EROFS_SUPER_OFFSET + 8]
                .try_into()
                .expect("4 bytes"),
        );
        assert_ne!(stored, 0);
        let mut verify = buf.clone();
        verify[EROFS_SUPER_OFFSET + 4..EROFS_SUPER_OFFSET + 8].fill(0);
        let recomputed = !crc32c::crc32c(&verify[EROFS_SUPER_OFFSET..BLOCK_SIZE as usize]);
        assert_eq!(stored, recomputed);
    }

    #[test]
    fn epoch_stored_correctly() {
        // ARRANGE
        let mut buf = vec![0u8; BLOCK_SIZE as usize];
        let mut p = test_params();
        p.epoch = 1_700_000_000;

        // ACT
        write_superblock(&mut buf, &p);

        // ASSERT
        let stored = u64::from_le_bytes(
            buf[EROFS_SUPER_OFFSET + 0x18..EROFS_SUPER_OFFSET + 0x20]
                .try_into()
                .expect("8 bytes"),
        );
        assert_eq!(stored, 1_700_000_000);
    }

    #[test]
    fn rootnid_stored() {
        // ARRANGE
        let mut buf = vec![0u8; BLOCK_SIZE as usize];
        let mut p = test_params();
        p.root_nid = 42;

        // ACT
        write_superblock(&mut buf, &p);

        // ASSERT
        let nid = u16::from_le_bytes(
            buf[EROFS_SUPER_OFFSET + 0x0E..EROFS_SUPER_OFFSET + 0x10]
                .try_into()
                .expect("2 bytes"),
        );
        assert_eq!(nid, 42);
    }
}
