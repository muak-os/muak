//! Directory-parent lookup and serialized directory entry ordering helpers.

use alloc::borrow::ToOwned as _;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString as _};
use alloc::vec::Vec;
use std::path::Path;

use crate::checked::align_up;
use crate::dir::{EROFS_FT_DIR, Entry};
use crate::layout::InodeLayout;

pub(super) fn find_parent_nid(
    inode: &InodeLayout,
    all_inodes: &[InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
) -> u64 {
    if inode.rel_path == "/" {
        return inode.nid;
    }
    let parent_rel = if let Some(parent) = Path::new(&inode.rel_path).parent() {
        let parent_rel_path = parent.to_string_lossy().to_string();
        if parent_rel_path.is_empty() {
            "/".to_owned()
        } else {
            parent_rel_path
        }
    } else {
        "/".to_owned()
    };

    path_to_idx.get(&parent_rel).map_or(0, |&index| {
        all_inodes.get(index).map_or(0, |inode| inode.nid)
    })
}

pub(super) fn align8(val: usize) -> usize {
    align_up(val, 8).unwrap_or(val)
}

pub(super) fn sorted_entries(
    inode: &InodeLayout,
    all_inodes: &[InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
    parent_nid: u64,
) -> Vec<Entry> {
    let mut entries = vec![
        Entry {
            name: b".".to_vec(),
            nid: inode.nid,
            file_type: EROFS_FT_DIR,
        },
        Entry {
            name: b"..".to_vec(),
            nid: parent_nid,
            file_type: EROFS_FT_DIR,
        },
    ];

    for child_rel in &inode.children {
        let Some(&child_index) = path_to_idx.get(child_rel) else {
            continue;
        };
        let Some(child) = all_inodes.get(child_index) else {
            continue;
        };
        let name = Path::new(child_rel)
            .file_name()
            .map(|name| name.to_string_lossy().as_bytes().to_vec())
            .unwrap_or_default();
        entries.push(Entry {
            name,
            nid: child.nid,
            file_type: child.file_type,
        });
    }

    entries.sort_by(|left_entry, right_entry| left_entry.name.cmp(&right_entry.name));
    entries
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;
    use std::io;

    use super::{find_parent_nid, sorted_entries};
    use crate::dir::{EROFS_FT_DIR, EROFS_FT_REG_FILE};
    use crate::inode::EROFS_INODE_FLAT_PLAIN;
    use crate::layout::{self, InodeLayout};
    use crate::source::SizedFile;
    use crate::testutil::test_config;
    use crate::tree::TreeEntry;

    #[test]
    fn find_parent_nid_for_root() {
        // ARRANGE
        let files = &mut [SizedFile {
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
        }];
        let cfg = test_config(1);

        // ACT
        let planned = layout::plan(files, &cfg).expect("plan");
        let inodes = &planned.inodes;
        let root = inodes.first().expect("root inode");

        // ASSERT
        assert_eq!(find_parent_nid(root, inodes, &BTreeMap::new()), root.nid);
    }

    #[test]
    fn find_parent_nid_for_nested_file() {
        // ARRANGE
        let mut file_data = [0_u8; 7];
        let mut file_cursor = io::Cursor::new(file_data.as_mut_slice());
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
                    rel_path: "/subdir/file.txt".to_owned(),
                    file_type: EROFS_FT_REG_FILE,
                    size: 7,
                    mode: 0o644,
                    uid: 0,
                    gid: 0,
                    mtime: 0,
                    mtime_nsec: 0,
                    symlink_target: vec![],
                    rdev: 0,
                },
                reader: &mut file_cursor,
            },
        ];
        let cfg = test_config(1);

        // ACT
        let planned = layout::plan(files, &cfg).expect("plan");
        let inodes = &planned.inodes;
        let path_to_idx: BTreeMap<_, _> = inodes
            .iter()
            .enumerate()
            .map(|(index, inode)| (inode.rel_path.clone(), index))
            .collect();
        let file = inodes
            .iter()
            .find(|inode| inode.rel_path == "/subdir/file.txt")
            .expect("found");
        let subdir = inodes
            .iter()
            .find(|inode| inode.rel_path == "/subdir")
            .expect("found");

        // ASSERT
        assert_eq!(find_parent_nid(file, inodes, &path_to_idx), subdir.nid);
    }

    #[test]
    fn build_sorted_dir_entries_smoke() {
        // ARRANGE
        let mut d1 = [0_u8; 1];
        let mut d2 = [0_u8; 1];
        let mut d3 = [0_u8; 1];
        let mut c1 = io::Cursor::new(d1.as_mut_slice());
        let mut c2 = io::Cursor::new(d2.as_mut_slice());
        let mut c3 = io::Cursor::new(d3.as_mut_slice());
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
                    rel_path: "/z".to_owned(),
                    file_type: EROFS_FT_REG_FILE,
                    size: 1,
                    mode: 0o644,
                    uid: 0,
                    gid: 0,
                    mtime: 0,
                    mtime_nsec: 0,
                    symlink_target: vec![],
                    rdev: 0,
                },
                reader: &mut c1,
            },
            SizedFile {
                entry: TreeEntry {
                    rel_path: "/a".to_owned(),
                    file_type: EROFS_FT_REG_FILE,
                    size: 1,
                    mode: 0o644,
                    uid: 0,
                    gid: 0,
                    mtime: 0,
                    mtime_nsec: 0,
                    symlink_target: vec![],
                    rdev: 0,
                },
                reader: &mut c2,
            },
            SizedFile {
                entry: TreeEntry {
                    rel_path: "/m".to_owned(),
                    file_type: EROFS_FT_REG_FILE,
                    size: 1,
                    mode: 0o644,
                    uid: 0,
                    gid: 0,
                    mtime: 0,
                    mtime_nsec: 0,
                    symlink_target: vec![],
                    rdev: 0,
                },
                reader: &mut c3,
            },
        ];
        let cfg = test_config(1);

        // ACT
        let planned = layout::plan(files, &cfg).expect("plan");
        let inodes = &planned.inodes;
        let path_to_idx: BTreeMap<_, _> = inodes
            .iter()
            .enumerate()
            .map(|(index, inode)| (inode.rel_path.clone(), index))
            .collect();
        let root = inodes.first().expect("root inode");
        let entries = sorted_entries(root, inodes, &path_to_idx, root.nid);

        // ASSERT
        assert_eq!(entries.len(), 5);
        assert_eq!(entries.first().expect("dot entry").name, b".");
        assert_eq!(entries.get(1).expect("dotdot entry").name, b"..");
        assert_eq!(entries.get(2).expect("a entry").name, b"a");
        assert_eq!(entries.get(3).expect("m entry").name, b"m");
        assert_eq!(entries.get(4).expect("z entry").name, b"z");
    }

    #[test]
    fn find_parent_nid_returns_zero_for_missing_parent() {
        // ARRANGE
        let inode = InodeLayout {
            rel_path: "/child".to_owned(),
            nid: 2,
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
            raw_data: Vec::new(),
            data_blkaddr: 0,
            data_blocks: 0,
            children: vec!["/missing".to_owned()],
            symlink_target: Vec::new(),
            rdev: 0,
            compressed: None,
        };

        // ACT
        let parent_nid = find_parent_nid(&inode, &[], &BTreeMap::new());
        let dir_len = sorted_entries(&inode, &[], &BTreeMap::new(), 9).len();

        // ASSERT
        assert_eq!(parent_nid, 0);
        assert_eq!(dir_len, 2);
    }

    #[test]
    fn build_sorted_dir_entries_skips_missing_and_keeps_known_children() {
        // ARRANGE
        let inode = InodeLayout {
            rel_path: "child".to_owned(),
            nid: 2,
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
            raw_data: Vec::new(),
            data_blkaddr: 0,
            data_blocks: 0,
            children: vec!["/missing-child".to_owned(), "/known".to_owned()],
            symlink_target: Vec::new(),
            rdev: 0,
            compressed: None,
        };
        let known_child = InodeLayout {
            rel_path: "/known".to_owned(),
            nid: 9,
            file_type: EROFS_FT_REG_FILE,
            ..inode.clone()
        };
        let mut path_to_idx = BTreeMap::new();
        path_to_idx.insert("/known".to_owned(), 1);

        // ACT
        let dir_entries = sorted_entries(&inode, &[inode.clone(), known_child], &path_to_idx, 7);

        // ASSERT
        assert_eq!(dir_entries.len(), 3);
        assert!(dir_entries.iter().any(|entry| entry.name == b"known"));
    }
}
