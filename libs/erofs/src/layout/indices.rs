//! Index structures for mapping paths to inodes and tracking parent-child relationships.

use alloc::collections::{BTreeMap, VecDeque};

use super::parent_rel;
use super::types::InodeLayout;
use crate::dir::EROFS_FT_DIR;
use crate::tree::TreeEntry;

/// Lookup maps populated from a directory tree walk.
pub struct LayoutIndices {
    pub path_to_idx: BTreeMap<String, usize>,
    pub dir_children: BTreeMap<String, Vec<String>>,
    pub nlink_map: BTreeMap<String, u16>,
}

/// Populate `LayoutIndices` from [`TreeEntry`] entries.
pub fn build_from_entries(entries: &[TreeEntry], inodes: &[InodeLayout]) -> LayoutIndices {
    let mut idx = LayoutIndices {
        path_to_idx: BTreeMap::new(),
        dir_children: BTreeMap::new(),
        nlink_map: BTreeMap::new(),
    };

    for (i, entry) in entries.iter().enumerate() {
        let rel = &entry.rel_path;
        idx.path_to_idx.insert(rel.clone(), i);
        let p_rel = parent_rel(rel);

        if *rel != "/" {
            idx.dir_children
                .entry(p_rel.clone())
                .or_default()
                .push(rel.clone());
        }

        let Some(inode) = inodes.get(i) else {
            continue;
        };

        if inode.file_type != EROFS_FT_DIR {
            continue;
        }
        idx.nlink_map.entry(rel.clone()).or_insert(2);
        if *rel != "/" {
            let parent_nlink = idx.nlink_map.entry(p_rel).or_insert(2);
            *parent_nlink = parent_nlink.saturating_add(1);
        }
    }
    idx
}

/// Write computed nlink counts into each directory inode.
pub fn apply_nlinks(
    inodes: &mut [InodeLayout],
    nlink_map: &BTreeMap<String, u16>,
    path_to_idx: &BTreeMap<String, usize>,
) {
    for (rel, &count) in nlink_map {
        let Some(&idx) = path_to_idx.get(rel) else {
            continue;
        };
        let Some(inode) = inodes.get_mut(idx) else {
            continue;
        };
        inode.nlink = count;
    }
}

/// Write child lists into each directory inode.
pub fn apply_children(
    inodes: &mut [InodeLayout],
    dir_children: &BTreeMap<String, Vec<String>>,
    path_to_idx: &BTreeMap<String, usize>,
) {
    for (parent_rel, children) in dir_children {
        let Some(&idx) = path_to_idx.get(parent_rel) else {
            continue;
        };
        let Some(inode) = inodes.get_mut(idx) else {
            continue;
        };
        inode.children.clone_from(children);
    }
}

/// Assign sequential inode numbers in BFS order matching NID assignment.
pub fn assign_inos(
    inodes: &mut [InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
    dir_children: &BTreeMap<String, Vec<String>>,
) {
    let mut ino: u32 = 1;

    if let Some(&root_idx) = path_to_idx.get("/")
        && let Some(root_inode) = inodes.get_mut(root_idx)
    {
        root_inode.ino = ino;
        ino = ino.saturating_add(1);
    }

    let mut bfs_queue = VecDeque::new();
    bfs_queue.push_back("/".to_owned());

    while let Some(dir_rel) = bfs_queue.pop_front() {
        let Some(sorted_children) = dir_children.get(&dir_rel) else {
            continue;
        };

        for child_rel in sorted_children {
            ino = set_ino(inodes, path_to_idx, child_rel, ino);
        }

        bfs_queue.extend(
            sorted_children
                .iter()
                .filter(|child_rel| is_dir(inodes, path_to_idx, child_rel))
                .cloned(),
        );
    }
}

/// Check whether the inode at the given relative path is a directory.
fn is_dir(inodes: &[InodeLayout], path_to_idx: &BTreeMap<String, usize>, rel: &str) -> bool {
    path_to_idx
        .get(rel)
        .and_then(|&idx| inodes.get(idx))
        .is_some_and(|inode| inode.file_type == EROFS_FT_DIR)
}

/// Set the inode number of a single inode and return the next number.
fn set_ino(
    inodes: &mut [InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
    rel: &str,
    ino: u32,
) -> u32 {
    let Some(&idx) = path_to_idx.get(rel) else {
        return ino;
    };
    let Some(inode) = inodes.get_mut(idx) else {
        return ino;
    };
    inode.ino = ino;
    ino.saturating_add(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dir::{EROFS_FT_DIR, EROFS_FT_REG_FILE};
    use crate::inode::EROFS_INODE_FLAT_PLAIN;
    use crate::layout::collect::initial_inodes;
    use crate::testutil::test_config;

    #[test]
    fn build_indices_mixed_types() {
        // ARRANGE
        let entries = vec![
            TreeEntry {
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
            TreeEntry {
                rel_path: "/file".to_owned(),
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
            TreeEntry {
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
        ];
        let inodes = initial_inodes(&entries, &test_config(0)).expect("inodes");

        // ACT
        let idx = build_from_entries(&entries, &inodes);

        // ASSERT
        assert_eq!(idx.path_to_idx.len(), 3);
        assert!(idx.dir_children.contains_key("/"));
    }

    #[test]
    fn assign_inos_handles_all_paths() {
        // ARRANGE
        let entries = vec![
            TreeEntry {
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
            TreeEntry {
                rel_path: "/file".to_owned(),
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
            TreeEntry {
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
        ];
        let mut inodes = initial_inodes(&entries, &test_config(0)).expect("inodes");
        let idx = build_from_entries(&entries, &inodes);

        // ACT
        assign_inos(&mut inodes, &idx.path_to_idx, &idx.dir_children);

        // ASSERT
        for inode in &inodes {
            assert!(inode.ino >= 1);
        }
    }

    #[test]
    fn set_ino_missing_path_returns_unchanged() {
        // ARRANGE
        let mut inodes = vec![];
        let path_to_idx: BTreeMap<String, usize> = BTreeMap::new();

        // ACT
        let result = set_ino(&mut inodes, &path_to_idx, "/nonexistent", 5);

        // ASSERT
        assert_eq!(result, 5);
    }

    #[test]
    fn apply_nlinks_updates_nlink_count() {
        // ARRANGE
        let mut inodes = vec![InodeLayout {
            rel_path: "/".to_owned(),
            nid: 36,
            ino: 0,
            mode: 0o40755,
            uid: 0,
            gid: 0,
            mtime: 0,
            mtime_nsec: 0,
            nlink: 0,
            file_type: EROFS_FT_DIR,
            size: 0,
            datalayout: EROFS_INODE_FLAT_PLAIN,
            xattr_payload: vec![],
            xattr_icount: 0,
            raw_data: Vec::new(),
            data_blkaddr: 0,
            data_blocks: 0,
            children: vec![],
            symlink_target: vec![],
            rdev: 0,
            compressed: None,
        }];
        let path_to_idx: BTreeMap<String, usize> = [("/".to_owned(), 0)].into_iter().collect();
        let nlink_map: BTreeMap<String, u16> = [("/".to_owned(), 2)].into_iter().collect();

        // ACT
        apply_nlinks(&mut inodes, &nlink_map, &path_to_idx);

        // ASSERT
        assert_eq!(inodes.first().expect("root inode").nlink, 2);
    }

    #[test]
    fn apply_children_populates_children_list() {
        // ARRANGE
        let mut inodes = vec![InodeLayout {
            rel_path: "/".to_owned(),
            nid: 36,
            ino: 0,
            mode: 0o40755,
            uid: 0,
            gid: 0,
            mtime: 0,
            mtime_nsec: 0,
            nlink: 2,
            file_type: EROFS_FT_DIR,
            size: 0,
            datalayout: EROFS_INODE_FLAT_PLAIN,
            xattr_payload: vec![],
            xattr_icount: 0,
            raw_data: Vec::new(),
            data_blkaddr: 0,
            data_blocks: 0,
            children: vec![],
            symlink_target: vec![],
            rdev: 0,
            compressed: None,
        }];
        let path_to_idx: BTreeMap<String, usize> = [("/".to_owned(), 0)].into_iter().collect();
        let dir_children: BTreeMap<String, Vec<String>> =
            [("/".to_owned(), vec!["/child".to_owned()])]
                .into_iter()
                .collect();

        // ACT
        apply_children(&mut inodes, &dir_children, &path_to_idx);

        // ASSERT
        let root = inodes.first().expect("root inode");
        assert_eq!(root.children.len(), 1);
        assert_eq!(root.children.first().expect("child path"), "/child");
    }

    #[test]
    fn assign_inos_with_nested_directories() {
        // ARRANGE
        let entries = vec![
            TreeEntry {
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
            TreeEntry {
                rel_path: "/a".to_owned(),
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
            TreeEntry {
                rel_path: "/a/b".to_owned(),
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
            TreeEntry {
                rel_path: "/a/b/c".to_owned(),
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
            TreeEntry {
                rel_path: "/a/b/c/file".to_owned(),
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
        ];
        let mut inodes = initial_inodes(&entries, &test_config(0)).expect("inodes");
        let idx = build_from_entries(&entries, &inodes);

        // ACT
        assign_inos(&mut inodes, &idx.path_to_idx, &idx.dir_children);

        // ASSERT
        for inode in &inodes {
            assert!(inode.ino >= 1);
        }
    }

    #[test]
    fn build_indices_creates_nlink_map() {
        // ARRANGE
        let entries = vec![
            TreeEntry {
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
            TreeEntry {
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
            TreeEntry {
                rel_path: "/subdir/file".to_owned(),
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
        ];
        let inodes = initial_inodes(&entries, &test_config(0)).expect("inodes");

        // ACT
        let idx = build_from_entries(&entries, &inodes);

        // ASSERT
        assert!(idx.nlink_map.contains_key("/"));
        assert!(idx.nlink_map.contains_key("/subdir"));
    }

    #[test]
    fn build_indices_ignores_missing_inode_entries() {
        // ARRANGE
        let entries = vec![
            super::TreeEntry {
                rel_path: "/".to_owned(),
                file_type: EROFS_FT_DIR,
                size: 0,
                mode: 0,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: Vec::new(),
                rdev: 0,
            },
            super::TreeEntry {
                rel_path: "/file".to_owned(),
                file_type: EROFS_FT_REG_FILE,
                size: 0,
                mode: 0,
                uid: 0,
                gid: 0,
                mtime: 0,
                mtime_nsec: 0,
                symlink_target: Vec::new(),
                rdev: 0,
            },
        ];
        let inodes = vec![InodeLayout {
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
            raw_data: Vec::new(),
            data_blkaddr: 0,
            data_blocks: 0,
            children: Vec::new(),
            symlink_target: Vec::new(),
            rdev: 0,
            compressed: None,
        }];

        // ACT
        let indices = build_from_entries(&entries, &inodes);

        // ASSERT
        assert!(indices.path_to_idx.contains_key("/file"));
        assert!(!indices.nlink_map.contains_key("/file"));
    }

    #[test]
    fn apply_children_and_nlinks_ignore_missing_indices() {
        // ARRANGE
        let mut inodes = vec![InodeLayout {
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
            raw_data: Vec::new(),
            data_blkaddr: 0,
            data_blocks: 0,
            children: Vec::new(),
            symlink_target: Vec::new(),
            rdev: 0,
            compressed: None,
        }];
        let path_to_idx = BTreeMap::new();
        let dir_children = BTreeMap::from([("/missing".to_owned(), vec!["/child".to_owned()])]);
        let nlink_map = BTreeMap::from([("/missing".to_owned(), 3_u16)]);

        // ACT
        apply_children(&mut inodes, &dir_children, &path_to_idx);
        apply_nlinks(&mut inodes, &nlink_map, &path_to_idx);

        // ASSERT
        let root = inodes.first().expect("root inode");
        assert!(root.children.is_empty());
        assert_eq!(root.nlink, 1);
    }
}
