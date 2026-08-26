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
    use std::io;

    use super::write_header;
    use crate::SLOT_SIZE;
    use crate::compress;
    use crate::dir::{EROFS_FT_DIR, EROFS_FT_REG_FILE};
    use crate::inode::{EROFS_INODE_COMPRESSED_COMPACT, EROFS_INODE_FLAT_PLAIN};
    use crate::layout::{self, InodeLayout};
    use crate::testutil::{compress_config, entry_of, test_config, zero_data};
    use crate::tree::TreeEntry;
    use crate::writer::image;

    fn root_entry() -> TreeEntry {
        TreeEntry {
            rel_path: "/".to_owned(),
            file_type: EROFS_FT_DIR,
            size: 0,
            mode: 0o40755,
            uid: 0,
            gid: 0,
            mtime: 0,
            mtime_nsec: 0,
            symlink_target: vec![],
            rdev: 0,
        }
    }

    fn reg_entry(rel_path: &str, size: u64) -> TreeEntry {
        TreeEntry {
            rel_path: rel_path.to_owned(),
            file_type: EROFS_FT_REG_FILE,
            size,
            mode: 0o644,
            uid: 0,
            gid: 0,
            mtime: 0,
            mtime_nsec: 0,
            symlink_target: vec![],
            rdev: 0,
        }
    }

    fn plan_from(entries: &[TreeEntry], cfg: &crate::MkfsConfig<'_>) -> layout::ImagePlan {
        let mut datas: Vec<Vec<u8>> = entries.iter().map(zero_data).collect();
        let mut cursors: Vec<io::Cursor<&mut [u8]>> = datas
            .iter_mut()
            .map(|data| io::Cursor::new(data.as_mut_slice()))
            .collect();
        let mut files = crate::testutil::pair_files(entries.to_vec(), &mut cursors);
        layout::plan(&mut files, cfg).expect("plan")
    }

    fn write_image(planned: &layout::ImagePlan, cfg: &crate::MkfsConfig<'_>) -> Vec<u8> {
        let entries: Vec<TreeEntry> = planned.inodes.iter().map(entry_of).collect();
        let datas: Vec<Vec<u8>> = entries.iter().map(zero_data).collect();
        let (mut meta_cursors, mut data_cursors) = crate::testutil::two_cursor_sets(&datas);
        let mut meta_files = crate::testutil::pair_files(entries.clone(), &mut meta_cursors);
        let mut data_files = crate::testutil::pair_files(entries, &mut data_cursors);

        let mut buf = Vec::new();
        image(&mut buf, planned, &mut meta_files, &mut data_files, cfg).expect("write");
        buf
    }

    #[test]
    fn compact_inode_at_correct_offset() {
        // ARRANGE
        let entries = &[root_entry(), reg_entry("/test", 4)];
        let cfg = test_config(0);

        // ACT
        let planned = plan_from(entries, &cfg);
        let buf = write_image(&planned, &cfg);
        let root_offset = 36 * SLOT_SIZE;
        let i_format = u16::from_le_bytes(
            buf.get(root_offset..root_offset + 2)
                .expect("root i_format bytes")
                .try_into()
                .expect("2 bytes"),
        );

        // ASSERT
        assert_eq!(i_format & 0x01, 0);
    }

    #[test]
    fn write_compressed_inode_has_compressed_compact_format() {
        // ARRANGE
        let entries = &[root_entry(), reg_entry("/zeros", 8192)];
        let cfg = compress_config(0);

        // ACT
        let planned = plan_from(entries, &cfg);
        let buf = write_image(&planned, &cfg);
        let file = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/zeros")
            .expect("found");
        let slot_off = usize::try_from(file.nid).expect("nid fits usize") * SLOT_SIZE;
        let i_format = u16::from_le_bytes(
            buf.get(slot_off..slot_off + 2)
                .expect("i_format bytes")
                .try_into()
                .expect("2b"),
        );
        let datalayout = (i_format >> 1) & 0x07;

        // ASSERT
        assert_eq!(datalayout, EROFS_INODE_COMPRESSED_COMPACT);
    }

    #[test]
    fn write_compressed_inode_i_u_is_pcluster_blocks() {
        // ARRANGE
        let entries = &[root_entry(), reg_entry("/zeros", 8192)];
        let cfg = compress_config(0);

        // ACT
        let planned = plan_from(entries, &cfg);
        let buf = write_image(&planned, &cfg);
        let file = planned
            .inodes
            .iter()
            .find(|inode| inode.rel_path == "/zeros")
            .expect("found");
        let slot_off = usize::try_from(file.nid).expect("nid fits usize") * SLOT_SIZE;
        let i_u = u32::from_le_bytes(
            buf.get(slot_off + 0x10..slot_off + 0x14)
                .expect("i_u bytes")
                .try_into()
                .expect("4b"),
        );
        let cf = file.compressed.as_ref().expect("compressed");

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
            data_blkaddr: 0,
            data_blocks: 0,
            children: Vec::new(),
            symlink_target: Vec::new(),
            rdev: 0x0501,
            compressed: None,
        };
        let mut image = vec![0_u8; 8192];
        let slot_offset = usize::try_from(inode.nid).expect("nid fits usize") * SLOT_SIZE;

        // ACT
        write_header(&mut image, &inode, slot_offset).expect("inode header");
        let stored = u32::from_le_bytes(
            image
                .get(slot_offset + 0x10..slot_offset + 0x14)
                .expect("rdev bytes")
                .try_into()
                .expect("4 bytes"),
        );

        // ASSERT
        assert_eq!(stored, 0x0501);
    }
}
