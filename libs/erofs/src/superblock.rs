//! On-disk superblock serialization.

use crate::BLOCK_SIZE;
use crate::checked::{write_byte, write_bytes};
use crate::error::{ErofsError, Result};
use crate::inode::Z_EROFS_COMPRESSION_ZSTD;

pub const EROFS_SUPER_MAGIC_V1: u32 = 0xE0F5_E1E2;
/// Byte offset of the superblock within the image (after the boot sector).
pub const EROFS_SUPER_OFFSET: usize = 1024;
const SB_OFF: u64 = 1024;
/// Superblock carries a CRC32-C checksum of itself.
pub const EROFS_FEATURE_COMPAT_SB_CHKSUM: u32 = 0x0000_0001;
/// Superblock carries a filesystem-wide modification timestamp.
pub const EROFS_FEATURE_COMPAT_MTIME: u32 = 0x0000_0002;
/// Last compressed pcluster is zero-padded to a full block boundary.
pub const EROFS_FEATURE_INCOMPAT_ZERO_PADDING: u32 = 0x0000_0001;
/// Superblock carries compression algorithm configuration.
pub const EROFS_FEATURE_INCOMPAT_COMPR_CFGS: u32 = 0x0000_0002;

/// Parameters for writing the on-disk superblock.
pub struct SuperblockParams {
    pub root_nid: u16,
    pub inos: u64,
    pub epoch: u64,
    pub blocks: u32,
    pub uuid: [u8; 16],
    pub has_compression: bool,
}

/// Serialize the 128-byte on-disk superblock into `writer` at the correct offset.
pub fn write(buf: &mut [u8], params: &SuperblockParams) -> Result<()> {
    let feature_compat = EROFS_FEATURE_COMPAT_SB_CHKSUM | EROFS_FEATURE_COMPAT_MTIME;
    let sb_off = usize::try_from(SB_OFF).unwrap_or_default();
    write_at(buf, sb_off, &EROFS_SUPER_MAGIC_V1.to_le_bytes())?;
    write_at(buf, sb_off.saturating_add(4), &0_u32.to_le_bytes())?;
    write_at(buf, sb_off.saturating_add(8), &feature_compat.to_le_bytes())?;
    write_byte_at(buf, sb_off.saturating_add(12), 12)?;
    write_byte_at(buf, sb_off.saturating_add(13), 0)?;
    write_at(
        buf,
        sb_off.saturating_add(14),
        &params.root_nid.to_le_bytes(),
    )?;
    write_at(buf, sb_off.saturating_add(16), &params.inos.to_le_bytes())?;
    write_at(buf, sb_off.saturating_add(24), &params.epoch.to_le_bytes())?;
    write_at(buf, sb_off.saturating_add(32), &0_u32.to_le_bytes())?;
    write_at(buf, sb_off.saturating_add(36), &params.blocks.to_le_bytes())?;
    write_at(buf, sb_off.saturating_add(40), &0_u32.to_le_bytes())?;
    write_at(buf, sb_off.saturating_add(44), &0_u32.to_le_bytes())?;
    write_at(buf, sb_off.saturating_add(48), &params.uuid)?;

    if params.has_compression {
        let feature_incompat =
            EROFS_FEATURE_INCOMPAT_ZERO_PADDING | EROFS_FEATURE_INCOMPAT_COMPR_CFGS;
        let available_compr_algs: u16 = 1 << Z_EROFS_COMPRESSION_ZSTD;
        write_at(
            buf,
            sb_off.saturating_add(80),
            &feature_incompat.to_le_bytes(),
        )?;
        write_at(
            buf,
            sb_off.saturating_add(84),
            &available_compr_algs.to_le_bytes(),
        )?;
        write_compr_cfgs(buf)?;
    }

    Ok(())
}

/// Write the zstd compression config after the superblock (`COMPR_CFGS` area).
fn write_compr_cfgs(buf: &mut [u8]) -> Result<()> {
    let cfg_offset = usize::try_from(SB_OFF.saturating_add(128)).unwrap_or_default();
    let cfg_size: u16 = 6;
    write_at(buf, cfg_offset, &cfg_size.to_le_bytes())?;
    write_byte_at(buf, cfg_offset.saturating_add(2), 0)?;
    write_byte_at(buf, cfg_offset.saturating_add(3), 5)?;

    Ok(())
}

/// Compute and write the CRC32-C checksum over the superblock region.
pub fn write_checksum(buf: &mut [u8]) -> Result<()> {
    let block_size = usize::try_from(BLOCK_SIZE).unwrap_or_default();
    let sb_off = usize::try_from(SB_OFF).unwrap_or_default();
    if buf.len() < block_size {
        return Err(ErofsError::Internal("superblock block out of bounds"));
    }
    write_at(buf, sb_off.saturating_add(4), &0_u32.to_le_bytes())?;
    let checksum_region = buf
        .get(sb_off..block_size)
        .ok_or(ErofsError::Internal("superblock block out of bounds"))?;
    let crc = !crc32c::crc32c(checksum_region);
    write_at(buf, sb_off.saturating_add(4), &crc.to_le_bytes())?;

    Ok(())
}

fn write_at(buf: &mut [u8], offset: usize, bytes: &[u8]) -> Result<()> {
    if write_bytes(buf, offset, bytes) {
        Ok(())
    } else {
        Err(ErofsError::Internal("superblock write out of bounds"))
    }
}

fn write_byte_at(buf: &mut [u8], offset: usize, value: u8) -> Result<()> {
    if write_byte(buf, offset, value) {
        Ok(())
    } else {
        Err(ErofsError::Internal("superblock write out of bounds"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inode::Z_EROFS_COMPRESSION_ZSTD;

    const TEST_BLOCK_SIZE: usize = 4096;

    fn test_params() -> SuperblockParams {
        SuperblockParams {
            root_nid: 0,
            inos: 1,
            epoch: 0,
            blocks: 1,
            uuid: [0_u8; 16],
            has_compression: false,
        }
    }

    fn make_buffer(size: usize) -> Vec<u8> {
        vec![0_u8; size]
    }

    #[test]
    fn magic_at_correct_offset() {
        // ARRANGE
        let mut buf = make_buffer(TEST_BLOCK_SIZE);

        // ACT
        write(&mut buf, &test_params()).expect("write");

        // ASSERT
        let magic = u32::from_le_bytes(
            buf.get(EROFS_SUPER_OFFSET..EROFS_SUPER_OFFSET + 4)
                .expect("magic bytes")
                .try_into()
                .expect("4 bytes"),
        );
        assert_eq!(magic, EROFS_SUPER_MAGIC_V1);
    }

    #[test]
    fn blkszbits_is_12() {
        // ARRANGE
        let mut buf = make_buffer(TEST_BLOCK_SIZE);

        // ACT
        write(&mut buf, &test_params()).expect("write");

        // ASSERT
        assert_eq!(
            *buf.get(EROFS_SUPER_OFFSET + 0x0C).expect("blkszbits byte"),
            12
        );
    }

    #[test]
    fn meta_blkaddr_is_zero() {
        // ARRANGE
        let mut buf = make_buffer(TEST_BLOCK_SIZE);

        // ACT
        write(&mut buf, &test_params()).expect("write");

        // ASSERT
        let meta = u32::from_le_bytes(
            buf.get(EROFS_SUPER_OFFSET + 0x28..EROFS_SUPER_OFFSET + 0x2C)
                .expect("meta block address bytes")
                .try_into()
                .expect("4 bytes"),
        );
        assert_eq!(meta, 0);
    }

    #[test]
    fn checksum_roundtrip() {
        // ARRANGE
        let mut buf = make_buffer(TEST_BLOCK_SIZE);
        write(&mut buf, &test_params()).expect("write");

        // ACT
        write_checksum(&mut buf).expect("checksum");

        // ASSERT
        let stored = u32::from_le_bytes(
            buf.get(EROFS_SUPER_OFFSET + 4..EROFS_SUPER_OFFSET + 8)
                .expect("checksum bytes")
                .try_into()
                .expect("4 bytes"),
        );
        assert_ne!(stored, 0);
        let mut verify = buf.clone();
        verify
            .get_mut(EROFS_SUPER_OFFSET + 4..EROFS_SUPER_OFFSET + 8)
            .expect("checksum bytes")
            .fill(0);
        let recomputed = !crc32c::crc32c(
            verify
                .get(EROFS_SUPER_OFFSET..TEST_BLOCK_SIZE)
                .expect("superblock bytes"),
        );
        assert_eq!(stored, recomputed);
    }

    #[test]
    fn epoch_stored_correctly() {
        // ARRANGE
        let mut buf = make_buffer(TEST_BLOCK_SIZE);
        let mut params = test_params();
        params.epoch = 1_700_000_000;

        // ACT
        write(&mut buf, &params).expect("write");

        // ASSERT
        let stored = u64::from_le_bytes(
            buf.get(EROFS_SUPER_OFFSET + 0x18..EROFS_SUPER_OFFSET + 0x20)
                .expect("epoch bytes")
                .try_into()
                .expect("8 bytes"),
        );
        assert_eq!(stored, 1_700_000_000);
    }

    #[test]
    fn rootnid_stored() {
        // ARRANGE
        let mut buf = make_buffer(TEST_BLOCK_SIZE);
        let mut params = test_params();
        params.root_nid = 42;

        // ACT
        write(&mut buf, &params).expect("write");

        // ASSERT
        let nid = u16::from_le_bytes(
            buf.get(EROFS_SUPER_OFFSET + 0x0E..EROFS_SUPER_OFFSET + 0x10)
                .expect("root nid bytes")
                .try_into()
                .expect("2 bytes"),
        );
        assert_eq!(nid, 42);
    }

    #[test]
    fn compression_flags_written_when_enabled() {
        // ARRANGE
        let mut buf = make_buffer(TEST_BLOCK_SIZE);
        let params = SuperblockParams {
            root_nid: 0,
            inos: 1,
            epoch: 0,
            blocks: 1,
            uuid: [0_u8; 16],
            has_compression: true,
        };

        // ACT
        write(&mut buf, &params).expect("write");

        // ASSERT
        let feature_incompat = u32::from_le_bytes(
            buf.get(EROFS_SUPER_OFFSET + 0x50..EROFS_SUPER_OFFSET + 0x54)
                .expect("feature incompat bytes")
                .try_into()
                .expect("4 bytes"),
        );
        assert_eq!(
            feature_incompat,
            EROFS_FEATURE_INCOMPAT_ZERO_PADDING | EROFS_FEATURE_INCOMPAT_COMPR_CFGS
        );
        let avail = u16::from_le_bytes(
            buf.get(EROFS_SUPER_OFFSET + 0x54..EROFS_SUPER_OFFSET + 0x56)
                .expect("available compressors bytes")
                .try_into()
                .expect("2 bytes"),
        );
        assert_eq!(avail, 1 << Z_EROFS_COMPRESSION_ZSTD);
    }

    #[test]
    fn compr_cfgs_written_correctly() {
        // ARRANGE
        let mut buf = make_buffer(TEST_BLOCK_SIZE);
        let params = SuperblockParams {
            root_nid: 0,
            inos: 1,
            epoch: 0,
            blocks: 1,
            uuid: [0_u8; 16],
            has_compression: true,
        };

        // ACT
        write(&mut buf, &params).expect("write");

        // ASSERT
        let cfg_offset = EROFS_SUPER_OFFSET + 128;
        let cfg_size = u16::from_le_bytes(
            buf.get(cfg_offset..cfg_offset + 2)
                .expect("compression config size bytes")
                .try_into()
                .expect("2 bytes"),
        );
        assert_eq!(cfg_size, 6);
        assert_eq!(
            *buf.get(cfg_offset + 2).expect("format byte"),
            0,
            "format byte"
        );
        assert_eq!(
            *buf.get(cfg_offset + 3).expect("windowlog byte"),
            5,
            "windowlog byte"
        );
    }

    #[test]
    fn no_compression_flags_when_disabled() {
        // ARRANGE
        let mut buf = make_buffer(TEST_BLOCK_SIZE);

        // ACT
        write(&mut buf, &test_params()).expect("write");

        // ASSERT
        let feature_incompat = u32::from_le_bytes(
            buf.get(EROFS_SUPER_OFFSET + 0x50..EROFS_SUPER_OFFSET + 0x54)
                .expect("feature incompat bytes")
                .try_into()
                .expect("4 bytes"),
        );
        assert_eq!(feature_incompat, 0);
    }
}
