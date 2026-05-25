//! Directory layout decisions and directory entry list construction.

use alloc::collections::BTreeMap;
use alloc::string::String;

use super::super::{parent_rel, types::InodeLayout};
use super::util::{header_only_padded, inline_fits, padded_slots, truncate_usize_to_u32};
use crate::dir::{self, DirEntry, EROFS_FT_DIR};
use crate::inode::{EROFS_INODE_FLAT_INLINE, EROFS_INODE_FLAT_PLAIN};

pub(super) fn layout(
    inodes: &mut [InodeLayout],
    i: usize,
    nid: u64,
    slot_offset: usize,
    inode_header: usize,
    path_to_idx: &BTreeMap<String, usize>,
    bs: usize,
) -> usize {
    let Some(children) = inodes.get(i).map(|inode| inode.children.clone()) else {
        return 0;
    };
    let dir_entries = build_entries(inodes, &children, path_to_idx, nid);
    let dir_data_size = dir::data_size(&dir_entries);
    let tail_size = dir_data_size.checked_rem(bs).unwrap_or_default();
    let full_blocks = dir_data_size.checked_div(bs).unwrap_or_default();

    let Some(inode) = inodes.get_mut(i) else {
        return 0;
    };
    inode.nid = nid;
    inode.size = truncate_usize_to_u32(dir_data_size);

    if dir_data_size > 0 && inline_fits(slot_offset, inode_header, dir_data_size, bs) {
        inode.datalayout = EROFS_INODE_FLAT_INLINE;
        inode.data_blocks = 0;
        padded_slots(inode_header, dir_data_size)
    } else if tail_size > 0 && inline_fits(slot_offset, inode_header, tail_size, bs) {
        inode.datalayout = EROFS_INODE_FLAT_INLINE;
        inode.data_blocks = truncate_usize_to_u32(full_blocks);
        padded_slots(inode_header, tail_size)
    } else {
        inode.datalayout = EROFS_INODE_FLAT_PLAIN;
        inode.data_blocks = truncate_usize_to_u32(dir_data_size.div_ceil(bs));
        header_only_padded(inode_header)
    }
}

pub(super) fn parent_nid(
    all_inodes: &[InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
    self_nid: u64,
) -> u64 {
    let dir_inode = all_inodes
        .iter()
        .find(|i| i.nid == self_nid && i.file_type == EROFS_FT_DIR);

    let Some(inode) = dir_inode else {
        return self_nid;
    };

    if inode.rel_path == "/" {
        return self_nid;
    }

    let parent_path = parent_rel(&inode.rel_path);
    path_to_idx
        .get(&parent_path)
        .and_then(|&parent_idx| all_inodes.get(parent_idx))
        .map_or(self_nid, |parent_inode| parent_inode.nid)
}

fn build_entries(
    all_inodes: &[InodeLayout],
    children: &[String],
    path_to_idx: &BTreeMap<String, usize>,
    self_nid: u64,
) -> Vec<DirEntry> {
    let parent_nid = parent_nid(all_inodes, path_to_idx, self_nid);

    let mut entries = vec![
        DirEntry {
            name: b".".to_vec(),
            nid: self_nid,
            file_type: EROFS_FT_DIR,
        },
        DirEntry {
            name: b"..".to_vec(),
            nid: parent_nid,
            file_type: EROFS_FT_DIR,
        },
    ];

    for child_rel in children {
        let Some(&idx) = path_to_idx.get(child_rel) else {
            continue;
        };
        let Some(child) = all_inodes.get(idx) else {
            continue;
        };
        let name = std::path::Path::new(child_rel)
            .file_name()
            .map(|n| n.to_string_lossy().as_bytes().to_vec())
            .unwrap_or_default();
        entries.push(DirEntry {
            name,
            nid: child.nid,
            file_type: child.file_type,
        });
    }

    entries.sort_by(|left, right| left.name.cmp(&right.name));
    entries
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;

    use super::{layout, parent_nid};
    use crate::Compression;
    use crate::dir::EROFS_FT_DIR;
    use crate::inode::{COMPACT_INODE_SIZE, EROFS_INODE_FLAT_INLINE, EROFS_INODE_FLAT_PLAIN};
    use crate::layout::InodeLayout;

    #[test]
    fn layout_dir_inline_single_block() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        for index in 0..5_u8 {
            std::fs::write(dir.path().join(format!("f{index}")), [index]).expect("write");
        }

        let inodes =
            crate::layout::plan(dir.path(), &crate::testutil::test_config(1)).expect("plan");
        let root = &inodes[0];

        // ACT
        // ASSERT
        assert_eq!(root.datalayout, EROFS_INODE_FLAT_INLINE);
        assert_eq!(root.data_blocks, 0);
    }

    #[test]
    fn layout_dir_inline_with_full_blocks() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        for index in 0..200_u8 {
            let name = format!("file_{index:03}.txt");
            std::fs::write(dir.path().join(&name), [index]).expect("write");
        }

        let inodes =
            crate::layout::plan(dir.path(), &crate::testutil::test_config(1)).expect("plan");
        let root = &inodes[0];

        // ACT
        // ASSERT
        assert!(root.data_blocks > 0);
    }

    #[test]
    fn layout_dir_flat_plain() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        for index in 0_u16..339 {
            let name = format!("file_{index:03}.txt");
            std::fs::write(dir.path().join(&name), [index as u8]).expect("write");
        }

        let inodes =
            crate::layout::plan(dir.path(), &crate::testutil::test_config(1)).expect("plan");
        let root = &inodes[0];

        // ACT
        // ASSERT
        assert_eq!(root.datalayout, EROFS_INODE_FLAT_PLAIN);
    }

    #[test]
    fn empty_directory_layout() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");

        let inodes =
            crate::layout::plan(dir.path(), &crate::testutil::test_config(1)).expect("plan");
        let root = &inodes[0];

        // ACT
        // ASSERT
        assert_eq!(root.datalayout, EROFS_INODE_FLAT_INLINE);
        assert_eq!(root.data_blocks, 0);
    }

    #[test]
    fn find_parent_nid_from_children_missing_indices_returns_self() {
        // ARRANGE
        let inodes = vec![InodeLayout {
            path: std::path::PathBuf::new(),
            rel_path: "/child".to_owned(),
            nid: 0,
            ino: 0,
            mode: 0,
            uid: 0,
            gid: 0,
            mtime: 0,
            mtime_nsec: 0,
            nlink: 1,
            file_type: EROFS_FT_DIR,
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
        }];

        // ACT
        // ASSERT
        assert_eq!(parent_nid(&inodes, &BTreeMap::new(), 7), 7);
    }

    #[test]
    fn layout_dir_returns_zero_for_missing_inode_index() {
        // ARRANGE
        let mut inodes = vec![InodeLayout {
            path: std::path::PathBuf::new(),
            rel_path: "/".to_owned(),
            nid: 0,
            ino: 0,
            mode: 0,
            uid: 0,
            gid: 0,
            mtime: 0,
            mtime_nsec: 0,
            nlink: 1,
            file_type: EROFS_FT_DIR,
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
        }];

        let advance = layout(
            &mut inodes,
            9,
            1,
            0,
            COMPACT_INODE_SIZE,
            &BTreeMap::new(),
            4096,
        );

        // ACT
        // ASSERT
        assert_eq!(advance, 0);
        let _ = Compression::None;
    }
}
