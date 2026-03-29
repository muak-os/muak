//! Index structures for mapping paths to inodes and tracking parent-child relationships.

use std::collections::BTreeMap;
use std::path::PathBuf;

use super::parent_rel;
use super::types::InodeLayout;
use crate::dir::EROFS_FT_DIR;

/// Lookup maps populated from a directory tree walk.
pub struct LayoutIndices {
    pub path_to_idx: BTreeMap<String, usize>,
    pub dir_children: BTreeMap<String, Vec<String>>,
    pub nlink_map: BTreeMap<String, u16>,
}

/// Populate `LayoutIndices` from the flat entries list.
pub fn build_indices(entries: &[(PathBuf, String)], inodes: &[InodeLayout]) -> LayoutIndices {
    let mut idx = LayoutIndices {
        path_to_idx: BTreeMap::new(),
        dir_children: BTreeMap::new(),
        nlink_map: BTreeMap::new(),
    };

    for (i, (_abs, rel)) in entries.iter().enumerate() {
        idx.path_to_idx.insert(rel.clone(), i);
        let p_rel = parent_rel(rel);

        if *rel != "/" {
            idx.dir_children
                .entry(p_rel.clone())
                .or_default()
                .push(rel.clone());
        }

        if inodes[i].file_type != EROFS_FT_DIR {
            continue;
        }
        idx.nlink_map.entry(rel.clone()).or_insert(2);
        if *rel != "/" {
            *idx.nlink_map.entry(p_rel).or_insert(2) += 1;
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
        if let Some(&idx) = path_to_idx.get(rel) {
            inodes[idx].nlink = count;
        }
    }
}

/// Write child lists into each directory inode.
pub fn apply_children(
    inodes: &mut [InodeLayout],
    dir_children: &BTreeMap<String, Vec<String>>,
    path_to_idx: &BTreeMap<String, usize>,
) {
    for (parent_rel, children) in dir_children {
        if let Some(&idx) = path_to_idx.get(parent_rel) {
            inodes[idx].children.clone_from(children);
        }
    }
}

/// Assign sequential inode numbers in BFS order matching NID assignment.
pub fn assign_inos(
    inodes: &mut [InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
    dir_children: &BTreeMap<String, Vec<String>>,
) {
    let mut ino: u32 = 1;

    if let Some(&root_idx) = path_to_idx.get("/") {
        inodes[root_idx].ino = ino;
        ino += 1;
    }

    let mut bfs_queue = std::collections::VecDeque::new();
    bfs_queue.push_back("/".to_string());

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
                .filter(|c| is_dir(inodes, path_to_idx, c))
                .cloned(),
        );
    }
}

/// Check whether the inode at the given relative path is a directory.
fn is_dir(inodes: &[InodeLayout], path_to_idx: &BTreeMap<String, usize>, rel: &str) -> bool {
    path_to_idx
        .get(rel)
        .is_some_and(|&idx| inodes[idx].file_type == EROFS_FT_DIR)
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
    inodes[idx].ino = ino;
    ino + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dir::EROFS_FT_DIR;
    use crate::inode::EROFS_INODE_FLAT_PLAIN;
    use crate::testutil::test_config;

    #[test]
    fn build_indices_mixed_types() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("file"), b"x").expect("write");
        std::fs::create_dir(dir.path().join("subdir")).expect("mkdir");
        let entries = crate::layout::collect::collect_entries(dir.path()).expect("entries");
        let inodes = crate::layout::collect::build_initial_inodes(&entries, &test_config(0))
            .expect("inodes");

        // ACT
        let idx = build_indices(&entries, &inodes);

        // ASSERT
        assert_eq!(idx.path_to_idx.len(), 3);
        assert!(idx.dir_children.contains_key("/"));
    }

    #[test]
    fn assign_inos_handles_all_paths() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("file"), b"x").expect("write");
        std::fs::create_dir(dir.path().join("subdir")).expect("mkdir");
        let entries = crate::layout::collect::collect_entries(dir.path()).expect("entries");
        let mut inodes = crate::layout::collect::build_initial_inodes(&entries, &test_config(0))
            .expect("inodes");
        let idx = build_indices(&entries, &inodes);

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
            path: std::path::PathBuf::from("/"),
            rel_path: "/".to_string(),
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
            inline_data: vec![],
            data_blkaddr: 0,
            data_blocks: 0,
            children: vec![],
            symlink_target: vec![],
            rdev: 0,
            compressed: None,
        }];
        let path_to_idx: BTreeMap<String, usize> = [("/".to_string(), 0)].into_iter().collect();
        let nlink_map: BTreeMap<String, u16> = [("/".to_string(), 2)].into_iter().collect();

        // ACT
        apply_nlinks(&mut inodes, &nlink_map, &path_to_idx);

        // ASSERT
        assert_eq!(inodes[0].nlink, 2);
    }

    #[test]
    fn apply_children_populates_children_list() {
        // ARRANGE
        let mut inodes = vec![InodeLayout {
            path: std::path::PathBuf::from("/"),
            rel_path: "/".to_string(),
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
            inline_data: vec![],
            data_blkaddr: 0,
            data_blocks: 0,
            children: vec![],
            symlink_target: vec![],
            rdev: 0,
            compressed: None,
        }];
        let path_to_idx: BTreeMap<String, usize> = [("/".to_string(), 0)].into_iter().collect();
        let dir_children: BTreeMap<String, Vec<String>> =
            [("/".to_string(), vec!["/child".to_string()])]
                .into_iter()
                .collect();

        // ACT
        apply_children(&mut inodes, &dir_children, &path_to_idx);

        // ASSERT
        assert_eq!(inodes[0].children.len(), 1);
        assert_eq!(inodes[0].children[0], "/child");
    }

    #[test]
    fn assign_inos_with_nested_directories() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("a/b/c")).expect("mkdir");
        std::fs::write(dir.path().join("a/b/c/file"), b"x").expect("write");
        let entries = crate::layout::collect::collect_entries(dir.path()).expect("entries");
        let mut inodes = crate::layout::collect::build_initial_inodes(&entries, &test_config(0))
            .expect("inodes");
        let idx = build_indices(&entries, &inodes);

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
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("subdir")).expect("mkdir");
        std::fs::write(dir.path().join("subdir/file"), b"x").expect("write");
        let entries = crate::layout::collect::collect_entries(dir.path()).expect("entries");
        let inodes = crate::layout::collect::build_initial_inodes(&entries, &test_config(0))
            .expect("inodes");

        // ACT
        let idx = build_indices(&entries, &inodes);

        // ASSERT
        assert!(idx.nlink_map.contains_key("/"));
        assert!(idx.nlink_map.contains_key("/subdir"));
    }
}
