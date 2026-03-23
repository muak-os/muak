//! Compact and extended inode on-disk serialization.

use crate::SLOT_SIZE;

/// Compact (32-byte) inode format discriminator bit.
pub const EROFS_INODE_LAYOUT_COMPACT: u16 = 0;
/// Flat data layout: all data in separate block(s), no inline tail.
pub const EROFS_INODE_FLAT_PLAIN: u16 = 0;
/// Flat inline layout: tail data packed immediately after the inode header.
pub const EROFS_INODE_FLAT_INLINE: u16 = 2;
/// Compressed inode with compacted variable-width indexes.
pub const EROFS_INODE_COMPRESSED_COMPACT: u16 = 3;
/// Size in bytes of a compact on-disk inode.
pub const COMPACT_INODE_SIZE: usize = 32;
/// Size of the `z_erofs_map_header` on-disk structure.
pub const Z_EROFS_MAP_HEADER_SIZE: usize = 8;
/// Zstandard compression algorithm identifier.
pub const Z_EROFS_COMPRESSION_ZSTD: u8 = 3;

/// Encode the `i_format` field for a compact inode.
pub fn encode_i_format_compact(datalayout: u16) -> u16 {
    (datalayout << 1) | EROFS_INODE_LAYOUT_COMPACT
}

/// Parameters for writing a compact inode.
pub struct CompactInodeParams {
    pub datalayout: u16,
    pub xattr_icount: u16,
    pub mode: u16,
    pub nlink: u16,
    pub size: u32,
    pub startblk_or_rdev: u32,
    pub ino: u32,
    pub uid: u16,
    pub gid: u16,
    pub reserved2: u32,
}

/// Serialize a 32-byte compact inode into the provided buffer.
pub fn write_compact_inode(buf: &mut [u8], p: &CompactInodeParams) {
    debug_assert!(buf.len() >= COMPACT_INODE_SIZE);
    let fmt = encode_i_format_compact(p.datalayout);
    buf[0x00..0x02].copy_from_slice(&fmt.to_le_bytes());
    buf[0x02..0x04].copy_from_slice(&p.xattr_icount.to_le_bytes());
    buf[0x04..0x06].copy_from_slice(&p.mode.to_le_bytes());
    buf[0x06..0x08].copy_from_slice(&p.nlink.to_le_bytes());
    buf[0x08..0x0C].copy_from_slice(&p.size.to_le_bytes());
    buf[0x0C..0x10].copy_from_slice(&0u32.to_le_bytes());
    buf[0x10..0x14].copy_from_slice(&p.startblk_or_rdev.to_le_bytes());
    buf[0x14..0x18].copy_from_slice(&p.ino.to_le_bytes());
    buf[0x18..0x1A].copy_from_slice(&p.uid.to_le_bytes());
    buf[0x1A..0x1C].copy_from_slice(&p.gid.to_le_bytes());
    buf[0x1C..0x20].copy_from_slice(&p.reserved2.to_le_bytes());
}

/// Compute the number of 32-byte slots an inode+xattr+inline_data occupies.
pub fn slot_count(inode_size: usize, xattr_size: usize, inline_size: usize) -> usize {
    let total = inode_size + xattr_size + inline_size;
    total.div_ceil(SLOT_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_inode_serialization() {
        // ARRANGE
        let mut buf = [0u8; 32];
        let params = CompactInodeParams {
            datalayout: EROFS_INODE_FLAT_INLINE,
            xattr_icount: 0,
            mode: 0o100644,
            nlink: 1,
            size: 42,
            startblk_or_rdev: u32::MAX,
            ino: 1,
            uid: 0,
            gid: 0,
            reserved2: 0,
        };

        // ACT
        write_compact_inode(&mut buf, &params);

        // ASSERT
        let fmt = u16::from_le_bytes(buf[0..2].try_into().expect("2 bytes"));
        assert_eq!(fmt & 0x01, 0);
        assert_eq!((fmt >> 1) & 0x07, EROFS_INODE_FLAT_INLINE);
        let mode = u16::from_le_bytes(buf[4..6].try_into().expect("2 bytes"));
        assert_eq!(mode, 0o100644);
        let size = u32::from_le_bytes(buf[8..12].try_into().expect("4 bytes"));
        assert_eq!(size, 42);
    }

    #[test]
    fn i_format_compact_encoding() {
        // ACT & ASSERT
        assert_eq!(encode_i_format_compact(EROFS_INODE_FLAT_PLAIN), 0x00);
        assert_eq!(encode_i_format_compact(EROFS_INODE_FLAT_INLINE), 0x04);
    }

    #[test]
    fn slot_count_calculation() {
        // ACT & ASSERT
        assert_eq!(slot_count(32, 0, 0), 1);
        assert_eq!(slot_count(32, 0, 10), 2);
        assert_eq!(slot_count(32, 0, 50), 3);
        assert_eq!(slot_count(64, 0, 0), 2);
    }
}
