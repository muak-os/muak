//! NID and data layout assignment for each inode in BFS order.

mod compact;
mod data;
mod dir;
mod file;
mod order;
mod util;

use alloc::collections::BTreeMap;

use super::types::InodeLayout;
use crate::checked::align_up;
use crate::dir::{EROFS_FT_DIR, EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
use crate::inode::COMPACT_INODE_SIZE;
use crate::superblock::EROFS_SUPER_OFFSET;
use crate::{Compression, SLOT_SIZE};

/// Byte offset at which the inode metadata region begins (no compression).
pub(super) const META_START: usize = EROFS_SUPER_OFFSET + 128;

/// Number of 16-byte ext slots reserved for compression config.
pub(super) const COMPR_CFG_EXTSLOTS: usize = 1;

/// Byte offset at which the inode metadata region begins.
pub(super) fn meta_start(has_compression: bool) -> usize {
    let base = if has_compression {
        META_START.saturating_add(COMPR_CFG_EXTSLOTS.saturating_mul(16))
    } else {
        META_START
    };
    align_up(base, SLOT_SIZE).unwrap_or(base)
}

/// Assign NIDs and decide data layout for each inode.
pub fn nids_and_layouts(
    inodes: &mut [InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
    compression: Compression,
) {
    let bs = util::block_size();
    let do_compress = compression.is_enabled();
    let mut meta_offset = meta_start(do_compress);
    let visit_order = order::bfs_order(inodes, path_to_idx);

    for i in visit_order {
        let slot_offset = meta_offset;
        let nid = meta_offset
            .checked_div(SLOT_SIZE)
            .map(util::truncate_usize_to_u64)
            .unwrap_or_default();
        let Some(inode) = inodes.get(i) else {
            continue;
        };
        let xattr_size = inode.xattr_payload.len();
        let inode_header = COMPACT_INODE_SIZE.saturating_add(xattr_size);

        let advance = match inode.file_type {
            EROFS_FT_DIR => dir::layout(inodes, i, nid, slot_offset, inode_header, path_to_idx, bs),
            EROFS_FT_SYMLINK => file::symlink(inodes, i, nid, slot_offset, inode_header, bs),
            EROFS_FT_REG_FILE => {
                file::regular(inodes, i, nid, slot_offset, inode_header, bs, compression)
            }
            _ => file::special(inodes, i, nid, inode_header),
        };
        meta_offset = meta_offset.saturating_add(advance);
    }
}

/// Assign data block addresses after all NIDs are computed.
pub fn data_block_addrs(inodes: &mut [InodeLayout], do_compress: bool) {
    data::data_block_addrs(inodes, do_compress);
}

/// Compute the total image size from the layout.
pub fn total_image_size(inodes: &[InodeLayout], do_compress: bool) -> usize {
    data::total_image_size(inodes, do_compress)
}

/// Compute compact index region layout from total logical cluster count and ebase.
pub(crate) fn index_layout(totalidx: usize, ebase: usize) -> (usize, usize, usize) {
    compact::index_layout(totalidx, ebase)
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;

    use super::{META_START, meta_start, nids_and_layouts};
    use crate::Compression;
    use crate::SLOT_SIZE;
    use crate::dir::EROFS_FT_DIR;
    use crate::inode::EROFS_INODE_FLAT_PLAIN;
    use crate::layout::InodeLayout;

    fn flat_plain_inode(rel_path: &str, file_type: u8) -> InodeLayout {
        InodeLayout {
            path: std::path::PathBuf::new(),
            rel_path: rel_path.to_string(),
            nid: 0,
            ino: 0,
            mode: 0,
            uid: 0,
            gid: 0,
            mtime: 0,
            mtime_nsec: 0,
            nlink: 1,
            file_type,
            size: 0,
            datalayout: EROFS_INODE_FLAT_PLAIN,
            xattr_payload: Vec::new(),
            xattr_icount: 0,
            inline_data: Vec::new(),
            data_blkaddr: 0,
            data_blocks: 0,
            children: Vec::new(),
            symlink_target: Vec::new(),
            rdev: 0,
            compressed: None,
        }
    }

    const FIRST_NID: u64 = (META_START / SLOT_SIZE) as u64;

    #[test]
    fn first_nid_is_36() {
        // ARRANGE
        // ACT
        // ASSERT
        assert_eq!(FIRST_NID, 36);
    }

    #[test]
    fn root_nid_is_36() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");

        let inodes =
            crate::layout::plan(dir.path(), &crate::testutil::test_config(1)).expect("plan");

        // ACT
        // ASSERT
        assert_eq!(inodes[0].nid, 36);
    }

    #[test]
    fn nids_assigned_contiguously() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a"), b"aaa").expect("write");
        std::fs::write(dir.path().join("b"), b"bbb").expect("write");

        let inodes =
            crate::layout::plan(dir.path(), &crate::testutil::test_config(1)).expect("plan");

        // ACT
        // ASSERT
        assert_eq!(inodes[0].nid, 36);
        assert!(inodes[1].nid > inodes[0].nid);
        assert!(inodes[2].nid > inodes[1].nid);
    }

    #[test]
    fn reference_nid_layout() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("hello.txt"), b"world").expect("write");
        std::os::unix::fs::symlink("/target", dir.path().join("link")).expect("symlink");
        std::fs::create_dir(dir.path().join("subdir")).expect("mkdir");
        std::fs::write(dir.path().join("subdir/world.txt"), b"hello").expect("write");

        let inodes =
            crate::layout::plan(dir.path(), &crate::testutil::test_config(0)).expect("plan");

        // ACT
        // ASSERT
        assert_eq!(inodes[0].nid, 36);
        assert_eq!(inodes[0].file_type, EROFS_FT_DIR);
        assert_eq!(
            inodes
                .iter()
                .find(|inode| inode.rel_path == "/hello.txt")
                .expect("hello")
                .nid,
            40
        );
        assert_eq!(
            inodes
                .iter()
                .find(|inode| inode.rel_path == "/link")
                .expect("link")
                .nid,
            42
        );
        assert_eq!(
            inodes
                .iter()
                .find(|inode| inode.rel_path == "/subdir")
                .expect("subdir")
                .nid,
            44
        );
        assert_eq!(
            inodes
                .iter()
                .find(|inode| inode.rel_path == "/subdir/world.txt")
                .expect("world")
                .nid,
            47
        );
    }

    #[test]
    fn compressed_root_nid_shifts_for_extslots() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("zeros"), vec![0_u8; 4096]).expect("write");

        let inodes =
            crate::layout::plan(dir.path(), &crate::testutil::compress_config(0)).expect("plan");

        let expected_nid = (meta_start(true) / SLOT_SIZE) as u64;
        // ACT
        // ASSERT
        assert_eq!(inodes[0].nid, expected_nid);
        assert!(inodes[0].nid > FIRST_NID);
    }

    #[test]
    fn meta_start_without_compression_is_aligned() {
        // ARRANGE
        let start = meta_start(false);

        // ACT
        // ASSERT
        assert_eq!(start % crate::SLOT_SIZE, 0);
        assert_eq!(
            start,
            META_START.div_ceil(crate::SLOT_SIZE) * crate::SLOT_SIZE
        );
    }

    #[test]
    fn meta_start_with_compression_is_larger() {
        // ARRANGE
        let without = meta_start(false);
        let with = meta_start(true);

        // ACT
        // ASSERT
        assert!(with > without);
        assert_eq!(with % crate::SLOT_SIZE, 0);
    }

    #[test]
    fn assign_nids_special_file_gets_flat_plain() {
        // ARRANGE
        let mut root = flat_plain_inode("/", EROFS_FT_DIR);
        root.children = vec!["/dev".to_string()];
        let mut special = flat_plain_inode("/dev", 3);
        special.rdev = 0x0501;

        let mut inodes = vec![root, special];
        let mut path_to_idx = BTreeMap::new();
        path_to_idx.insert("/".to_string(), 0);
        path_to_idx.insert("/dev".to_string(), 1);

        nids_and_layouts(&mut inodes, &path_to_idx, Compression::None);

        // ACT
        // ASSERT
        assert_eq!(inodes[1].datalayout, EROFS_INODE_FLAT_PLAIN);
        assert_ne!(inodes[1].nid, 0);
    }
}
