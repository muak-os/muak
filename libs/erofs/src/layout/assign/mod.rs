//! NID and data layout assignment for each inode in BFS order.

mod compact;
mod data;
mod dir;
mod file;
mod order;
pub(super) mod util;

use alloc::collections::BTreeMap;

use super::types::InodeLayout;
use crate::checked::align_up;
use crate::dir::{EROFS_FT_DIR, EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
use crate::error::Result;
use crate::inode::COMPACT_INODE_SIZE;
use crate::source::SizedFile;
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
///
/// # Errors
///
/// Returns an error when a regular file cannot be read.
pub fn nids_and_layouts(
    inodes: &mut [InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
    compression: Compression,
    files: &mut [SizedFile<'_>],
) -> Result<()> {
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
            EROFS_FT_REG_FILE => file::regular(
                inodes,
                i,
                nid,
                slot_offset,
                inode_header,
                bs,
                compression,
                files,
            )?,
            _ => file::special(inodes, i, nid, inode_header),
        };
        meta_offset = meta_offset.saturating_add(advance);
    }

    Ok(())
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
    use std::io;

    use super::nids_and_layouts;
    use crate::Compression;
    use crate::SLOT_SIZE;
    use crate::dir::{EROFS_FT_DIR, EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
    use crate::inode::EROFS_INODE_FLAT_PLAIN;
    use crate::layout::{ImagePlan, InodeLayout, plan};
    use crate::source::SizedFile;
    use crate::testutil::{compress_config, test_config};
    use crate::tree::TreeEntry;

    fn assert_reference_nids(planned: &ImagePlan) {
        let inodes = &planned.inodes;
        let root = inodes.first().expect("root inode");
        assert_eq!(root.nid, 36);
        assert_eq!(root.file_type, EROFS_FT_DIR);
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

    fn flat_plain_inode(rel_path: &str, file_type: u8) -> InodeLayout {
        InodeLayout {
            rel_path: rel_path.to_owned(),
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
            raw_data: Vec::new(),
            data_blkaddr: 0,
            data_blocks: 0,
            children: Vec::new(),
            symlink_target: Vec::new(),
            rdev: 0,
            compressed: None,
        }
    }

    const FIRST_NID: usize = super::META_START.div_euclid(SLOT_SIZE);

    #[test]
    fn first_nid_is_36() {
        // ARRANGE & ACT & ASSERT
        assert_eq!(FIRST_NID, 36);
    }

    #[test]
    fn root_nid_is_36() {
        // ARRANGE
        let entry = TreeEntry {
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
        };
        let files = &mut [SizedFile {
            entry,
            reader: &mut io::empty(),
        }];

        // ACT
        let planned = plan(files, &test_config(1)).expect("plan");
        let inodes = &planned.inodes;

        // ASSERT
        assert_eq!(inodes.first().expect("root inode").nid, 36);
    }

    #[test]
    fn nids_assigned_contiguously() {
        // ARRANGE
        let mut data_a = [0_u8; 3];
        let mut data_b = [0_u8; 3];
        let mut cursor_a = io::Cursor::new(data_a.as_mut_slice());
        let mut cursor_b = io::Cursor::new(data_b.as_mut_slice());
        let files = &mut [
            SizedFile {
                entry: TreeEntry {
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
                },
                reader: &mut io::empty(),
            },
            SizedFile {
                entry: TreeEntry {
                    rel_path: "/a".to_owned(),
                    file_type: EROFS_FT_REG_FILE,
                    size: 3,
                    mode: 0o644,
                    uid: 0,
                    gid: 0,
                    mtime: 0,
                    mtime_nsec: 0,
                    symlink_target: vec![],
                    rdev: 0,
                },
                reader: &mut cursor_a,
            },
            SizedFile {
                entry: TreeEntry {
                    rel_path: "/b".to_owned(),
                    file_type: EROFS_FT_REG_FILE,
                    size: 3,
                    mode: 0o644,
                    uid: 0,
                    gid: 0,
                    mtime: 0,
                    mtime_nsec: 0,
                    symlink_target: vec![],
                    rdev: 0,
                },
                reader: &mut cursor_b,
            },
        ];

        // ACT
        let planned = plan(files, &test_config(1)).expect("plan");
        let inodes = &planned.inodes;

        // ASSERT
        let root = inodes.first().expect("root inode");
        let first_child = inodes.get(1).expect("first child inode");
        let second_child = inodes.get(2).expect("second child inode");
        assert_eq!(root.nid, 36);
        assert!(first_child.nid > root.nid);
        assert!(second_child.nid > first_child.nid);
    }

    #[test]
    fn reference_nid_layout() {
        // ARRANGE / ACT
        let mut hello_data = [0_u8; 5];
        let mut world_data = [0_u8; 5];
        let mut hello_c = io::Cursor::new(hello_data.as_mut_slice());
        let mut world_c = io::Cursor::new(world_data.as_mut_slice());
        let mut files = [
            SizedFile {
                entry: TreeEntry {
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
                },
                reader: &mut io::empty(),
            },
            SizedFile {
                entry: TreeEntry {
                    rel_path: "/hello.txt".to_owned(),
                    file_type: EROFS_FT_REG_FILE,
                    size: 5,
                    mode: 0o644,
                    uid: 0,
                    gid: 0,
                    mtime: 0,
                    mtime_nsec: 0,
                    symlink_target: vec![],
                    rdev: 0,
                },
                reader: &mut hello_c,
            },
            SizedFile {
                entry: TreeEntry {
                    rel_path: "/link".to_owned(),
                    file_type: EROFS_FT_SYMLINK,
                    size: 0,
                    mode: 0o120_777,
                    uid: 0,
                    gid: 0,
                    mtime: 0,
                    mtime_nsec: 0,
                    symlink_target: b"/target".to_vec(),
                    rdev: 0,
                },
                reader: &mut io::empty(),
            },
            SizedFile {
                entry: TreeEntry {
                    rel_path: "/subdir".to_owned(),
                    file_type: EROFS_FT_DIR,
                    size: 0,
                    mode: 0o40755,
                    uid: 0,
                    gid: 0,
                    mtime: 0,
                    mtime_nsec: 0,
                    symlink_target: vec![],
                    rdev: 0,
                },
                reader: &mut io::empty(),
            },
            SizedFile {
                entry: TreeEntry {
                    rel_path: "/subdir/world.txt".to_owned(),
                    file_type: EROFS_FT_REG_FILE,
                    size: 5,
                    mode: 0o644,
                    uid: 0,
                    gid: 0,
                    mtime: 0,
                    mtime_nsec: 0,
                    symlink_target: vec![],
                    rdev: 0,
                },
                reader: &mut world_c,
            },
        ];
        let planned = plan(&mut files, &test_config(0)).expect("plan");

        // ASSERT
        assert_reference_nids(&planned);
    }

    #[test]
    fn compressed_root_nid_shifts_for_extslots() {
        // ARRANGE
        let mut zeros_data = vec![0_u8; 4096];
        let mut zeros_c = io::Cursor::new(zeros_data.as_mut_slice());
        let files = &mut [
            SizedFile {
                entry: TreeEntry {
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
                },
                reader: &mut io::empty(),
            },
            SizedFile {
                entry: TreeEntry {
                    rel_path: "/zeros".to_owned(),
                    file_type: EROFS_FT_REG_FILE,
                    size: 4096,
                    mode: 0o644,
                    uid: 0,
                    gid: 0,
                    mtime: 0,
                    mtime_nsec: 0,
                    symlink_target: vec![],
                    rdev: 0,
                },
                reader: &mut zeros_c,
            },
        ];

        // ACT
        let planned = plan(files, &compress_config(0)).expect("plan");
        let inodes = &planned.inodes;
        let expected_nid = u64::try_from(super::meta_start(true).div_euclid(SLOT_SIZE))
            .expect("expected nid fits u64");

        // ASSERT
        let root = inodes.first().expect("root inode");
        assert_eq!(root.nid, expected_nid);
        assert!(root.nid > u64::try_from(FIRST_NID).expect("first nid fits u64"));
    }

    #[test]
    fn meta_start_without_compression_is_aligned() {
        // ARRANGE
        let start = super::meta_start(false);

        // ACT & ASSERT
        assert!(start.is_multiple_of(crate::SLOT_SIZE));
        assert_eq!(
            start,
            super::META_START.div_ceil(crate::SLOT_SIZE) * crate::SLOT_SIZE
        );
    }

    #[test]
    fn meta_start_with_compression_is_larger() {
        // ARRANGE
        let without = super::meta_start(false);
        let with = super::meta_start(true);

        // ACT & ASSERT
        assert!(with > without);
        assert!(with.is_multiple_of(crate::SLOT_SIZE));
    }

    #[test]
    fn assign_nids_special_file_gets_flat_plain() {
        // ARRANGE
        let mut root = flat_plain_inode("/", EROFS_FT_DIR);
        root.children = vec!["/dev".to_owned()];
        let mut special = flat_plain_inode("/dev", 3);
        special.rdev = 0x0501;
        let mut inodes = vec![root, special];
        let mut path_to_idx = BTreeMap::new();
        path_to_idx.insert("/".to_owned(), 0);
        path_to_idx.insert("/dev".to_owned(), 1);
        let mut files = vec![
            SizedFile {
                entry: TreeEntry {
                    rel_path: "/".to_owned(),
                    file_type: EROFS_FT_DIR,
                    size: 0,
                    mode: 0,
                    uid: 0,
                    gid: 0,
                    mtime: 0,
                    mtime_nsec: 0,
                    symlink_target: vec![],
                    rdev: 0,
                },
                reader: Box::leak(Box::new(io::empty())),
            },
            SizedFile {
                entry: TreeEntry {
                    rel_path: "/dev".to_owned(),
                    file_type: 3,
                    size: 0,
                    mode: 0,
                    uid: 0,
                    gid: 0,
                    mtime: 0,
                    mtime_nsec: 0,
                    symlink_target: vec![],
                    rdev: 0,
                },
                reader: Box::leak(Box::new(io::empty())),
            },
        ];

        // ACT
        nids_and_layouts(&mut inodes, &path_to_idx, Compression::None, &mut files)
            .expect("nids_and_layouts");

        // ASSERT
        let special = inodes.get(1).expect("special inode");
        assert_eq!(special.datalayout, EROFS_INODE_FLAT_PLAIN);
        assert_ne!(special.nid, 0);
    }
}
