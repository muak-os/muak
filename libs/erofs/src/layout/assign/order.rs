//! Breadth-first inode traversal and child ordering helpers.

use alloc::collections::{BTreeMap, VecDeque};
use std::path::Path;

use super::super::types::InodeLayout;
use crate::dir::EROFS_FT_DIR;

pub(super) fn bfs_order(
    inodes: &[InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
) -> Vec<usize> {
    let mut order = Vec::with_capacity(inodes.len());
    let mut queue = VecDeque::new();

    let Some(&root_idx) = path_to_idx.get("/") else {
        return order;
    };
    order.push(root_idx);
    queue.push_back(root_idx);

    while let Some(dir_idx) = queue.pop_front() {
        let sorted = sorted_children(inodes, dir_idx);
        enqueue_children(&sorted, inodes, path_to_idx, &mut order, &mut queue);
    }
    order
}

fn sorted_children(inodes: &[InodeLayout], dir_idx: usize) -> Vec<String> {
    let Some(dir_inode) = inodes.get(dir_idx) else {
        return Vec::new();
    };

    let mut children = dir_inode.children.clone();
    children.sort_by(|left, right| {
        let left_name = Path::new(left)
            .file_name()
            .map(|file_name| file_name.to_string_lossy())
            .unwrap_or_default();
        let right_name = Path::new(right)
            .file_name()
            .map(|file_name| file_name.to_string_lossy())
            .unwrap_or_default();
        left_name.as_ref().cmp(right_name.as_ref())
    });
    children
}

fn enqueue_children(
    sorted: &[String],
    inodes: &[InodeLayout],
    path_to_idx: &BTreeMap<String, usize>,
    order: &mut Vec<usize>,
    queue: &mut VecDeque<usize>,
) {
    for child_rel in sorted {
        let Some(&idx) = path_to_idx.get(child_rel.as_str()) else {
            continue;
        };
        order.push(idx);
        if inodes
            .get(idx)
            .is_some_and(|inode| inode.file_type == EROFS_FT_DIR)
        {
            queue.push_back(idx);
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;

    use super::bfs_order;
    use crate::dir::EROFS_FT_DIR;
    use crate::inode::EROFS_INODE_FLAT_PLAIN;
    use crate::layout::InodeLayout;

    #[test]
    fn bfs_order_empty_path_to_idx_returns_empty() {
        // ARRANGE
        let inodes: Vec<InodeLayout> = Vec::new();
        let path_to_idx: BTreeMap<String, usize> = BTreeMap::new();

        // ACT
        let order = bfs_order(&inodes, &path_to_idx);

        // ASSERT
        assert!(order.is_empty());
    }

    #[test]
    fn bfs_order_ignores_missing_child_indices() {
        // ARRANGE
        let inode = InodeLayout {
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
            raw_data: Vec::new(),
            data_blkaddr: 0,
            data_blocks: 0,
            children: vec!["/missing".to_owned()],
            symlink_target: Vec::new(),
            rdev: 0,
            compressed: None,
        };
        let path_to_idx = BTreeMap::new();

        // ACT
        let order = bfs_order(&[inode], &path_to_idx);

        // ASSERT
        assert!(order.is_empty());
    }
}
