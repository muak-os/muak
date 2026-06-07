//! Compact inode header writing and xattr payload placement.

use crate::checked::{add, write_bytes};
use crate::dir::{EROFS_FT_DIR, EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
use crate::error::{ErofsError, Result};
use crate::inode::{self, COMPACT_INODE_SIZE, CompactInodeParams};
use crate::layout::InodeLayout;

pub(super) fn write_header(buf: &mut [u8], inode: &InodeLayout, slot_offset: usize) -> Result<()> {
    let i_u = if inode.compressed.is_some() {
        inode.data_blocks
    } else if inode.file_type != EROFS_FT_DIR
        && inode.file_type != EROFS_FT_REG_FILE
        && inode.file_type != EROFS_FT_SYMLINK
    {
        inode.rdev
    } else if inode.data_blocks > 0 {
        inode.data_blkaddr
    } else if inode.file_type == EROFS_FT_REG_FILE && inode.size == 0 {
        0
    } else {
        u32::MAX
    };

    let inode_header_end = add(slot_offset, COMPACT_INODE_SIZE)
        .ok_or(ErofsError::Internal("inode header write overflow"))?;

    let mut inode_buf = [0_u8; COMPACT_INODE_SIZE];
    inode::write_compact(
        &mut inode_buf,
        &CompactInodeParams {
            datalayout: inode.datalayout,
            xattr_icount: inode.xattr_icount,
            mode: inode.mode,
            nlink: inode.nlink,
            size: inode.size,
            startblk_or_rdev: i_u,
            ino: inode.ino,
            uid: inode.uid,
            gid: inode.gid,
            reserved2: 0,
        },
    );
    if !write_bytes(buf, slot_offset, &inode_buf) {
        return Err(ErofsError::Internal("inode header write out of bounds"));
    }

    if !inode.xattr_payload.is_empty() && !write_bytes(buf, inode_header_end, &inode.xattr_payload)
    {
        return Err(ErofsError::Internal("inode xattr write out of bounds"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::write_header;
    use crate::SLOT_SIZE;
    use crate::compress;
    use crate::inode::{EROFS_INODE_COMPRESSED_COMPACT, EROFS_INODE_FLAT_PLAIN};
    use crate::layout::collect::FilesystemTreeSource;
    use crate::layout::{self, InodeLayout};
    use crate::testutil::{compress_config, test_config};
    use crate::writer::write_image;

    #[test]
    fn compact_inode_at_correct_offset() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("test"), b"data").expect("write");
        let cfg = test_config(0);

        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
        let mut image = Vec::new();
        write_image(&mut image, &planned, &cfg).expect("write");

        let root_offset = 36 * SLOT_SIZE;
        let i_format = u16::from_le_bytes(
            image
                .get(root_offset..root_offset + 2)
                .expect("root i_format bytes")
                .try_into()
                .expect("2 bytes"),
        );
        // ACT
        // ASSERT
        assert_eq!(i_format & 0x01, 0);
    }

    #[test]
    fn write_compressed_inode_has_compressed_compact_format() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0_u8; 8192]).expect("write");
        let cfg = compress_config(0);

        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
        let mut image = Vec::new();
        write_image(&mut image, &planned, &cfg).expect("write");

        let file = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/zeros")
            .expect("found");
        let slot_off = usize::try_from(file.nid).expect("nid fits usize") * SLOT_SIZE;
        let i_format = u16::from_le_bytes(
            image
                .get(slot_off..slot_off + 2)
                .expect("i_format bytes")
                .try_into()
                .expect("2b"),
        );
        let datalayout = (i_format >> 1) & 0x07;
        // ACT
        // ASSERT
        assert_eq!(datalayout, EROFS_INODE_COMPRESSED_COMPACT);
    }

    #[test]
    fn write_compressed_inode_i_u_is_pcluster_blocks() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0_u8; 8192]).expect("write");
        let cfg = compress_config(0);

        let planned = layout::plan(&FilesystemTreeSource::new(dir.path()), &cfg).expect("plan");
        let mut image = Vec::new();
        write_image(&mut image, &planned, &cfg).expect("write");

        let file = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/zeros")
            .expect("found");
        let slot_off = usize::try_from(file.nid).expect("nid fits usize") * SLOT_SIZE;
        let i_u = u32::from_le_bytes(
            image
                .get(slot_off + 0x10..slot_off + 0x14)
                .expect("i_u bytes")
                .try_into()
                .expect("4b"),
        );
        let cf = file.compressed.as_ref().expect("compressed");
        // ACT
        // ASSERT
        assert_eq!(i_u, compress::pcluster_blocks(cf));
    }

    #[test]
    fn write_inode_header_rdev_for_special_file() {
        // ARRANGE
        let inode = InodeLayout {
            rel_path: "/dev/null".to_owned(),
            nid: 36,
            ino: 0,
            mode: 0o020_666,
            uid: 0,
            gid: 0,
            mtime: 0,
            mtime_nsec: 0,
            nlink: 1,
            file_type: 3,
            size: 0,
            datalayout: EROFS_INODE_FLAT_PLAIN,
            xattr_payload: Vec::new(),
            xattr_icount: 0,
            inline_data: Vec::new(),
            raw_data: Vec::new(),
            data_blkaddr: 0,
            data_blocks: 0,
            children: Vec::new(),
            symlink_target: Vec::new(),
            rdev: 0x0501,
            compressed: None,
        };
        let mut image = vec![0_u8; 8192];
        let slot_offset = usize::try_from(inode.nid).expect("nid fits usize") * SLOT_SIZE;

        write_header(&mut image, &inode, slot_offset).expect("inode header");

        let stored = u32::from_le_bytes(
            image
                .get(slot_offset + 0x10..slot_offset + 0x14)
                .expect("rdev bytes")
                .try_into()
                .expect("4 bytes"),
        );
        // ACT
        // ASSERT
        assert_eq!(stored, 0x0501);
    }
}
