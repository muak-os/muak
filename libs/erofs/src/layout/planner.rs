//! High-level layout planning pipeline from sized file entries to assigned inodes.

use super::ImagePlan;
use super::assign;
use super::collect;
use super::indices;
use crate::MkfsConfig;
use crate::error::Result;
use crate::source::SizedFile;
use crate::tree::TreeEntry;

/// Plan the full image layout from sized file entries.
pub fn plan(files: &mut [SizedFile<'_>], config: &MkfsConfig<'_>) -> Result<ImagePlan> {
    let entries: Vec<TreeEntry> = files.iter().map(|sized| sized.entry.clone()).collect();
    let mut inodes = collect::initial_inodes(&entries, config)?;
    let idx = indices::build_from_entries(&entries, &inodes);

    indices::apply_nlinks(&mut inodes, &idx.nlink_map, &idx.path_to_idx);
    indices::apply_children(&mut inodes, &idx.dir_children, &idx.path_to_idx);
    indices::assign_inos(&mut inodes, &idx.path_to_idx, &idx.dir_children);
    assign::nids_and_layouts(&mut inodes, &idx.path_to_idx, config.compression, files)?;
    assign::data_block_addrs(&mut inodes, config.compression.is_enabled());

    let total_size = assign::total_image_size(&inodes, config.compression.is_enabled());
    let do_compress = config.compression.is_enabled();

    Ok(ImagePlan {
        inodes,
        total_size,
        do_compress,
    })
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::plan;
    use crate::dir::{EROFS_FT_DIR, EROFS_FT_REG_FILE};
    use crate::source::SizedFile;
    use crate::testutil::test_config;
    use crate::tree::TreeEntry;

    #[test]
    fn readdir_order_nested_directories() {
        // ARRANGE
        let mut b_data = [0_u8; 1];
        let mut b_cursor = io::Cursor::new(b_data.as_mut_slice());
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
                    rel_path: "/a/b".to_owned(),
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
                reader: &mut b_cursor,
            },
        ];

        // ACT
        let plan = plan(files, &test_config(0)).expect("plan");

        // ASSERT
        assert!(plan.inodes.iter().any(|inode| inode.rel_path == "/"));
        assert!(plan.inodes.iter().any(|inode| inode.rel_path == "/a"));
        assert!(plan.inodes.iter().any(|inode| inode.rel_path == "/a/b"));
    }
}
