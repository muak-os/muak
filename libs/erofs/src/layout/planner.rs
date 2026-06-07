//! High-level layout planning pipeline from source tree to assigned inodes.

use std::fs;
use std::path::Path;

use super::ImagePlan;
use super::assign;
use super::collect;
use super::indices;
use crate::MkfsConfig;
use crate::dir::EROFS_FT_REG_FILE;
use crate::error::{ErofsError, Result};

/// Plan the full image layout from a source directory.
pub fn plan(source_dir: &Path, config: &MkfsConfig<'_>) -> Result<ImagePlan> {
    if !source_dir.is_dir() {
        return Err(ErofsError::InvalidSource(source_dir.to_path_buf()));
    }

    let entries = collect::entries(source_dir)?;
    let mut inodes = collect::initial_inodes(&entries, config)?;
    let idx = indices::build(&entries, &inodes);

    indices::apply_nlinks(&mut inodes, &idx.nlink_map, &idx.path_to_idx);
    indices::apply_children(&mut inodes, &idx.dir_children, &idx.path_to_idx);
    indices::assign_inos(&mut inodes, &idx.path_to_idx, &idx.dir_children);
    assign::nids_and_layouts(&mut inodes, &idx.path_to_idx, config.compression);
    assign::data_block_addrs(&mut inodes, config.compression.is_enabled());

    // Read file contents for uncompressed regular files so the writer does
    // not need to touch the filesystem during emission.
    for inode in &mut inodes {
        if inode.file_type != EROFS_FT_REG_FILE || inode.size == 0 || inode.compressed.is_some() {
            continue;
        }
        let data = fs::read(&inode.path).map_err(|e| {
            ErofsError::Io(std::io::Error::new(
                e.kind(),
                format!("{}: {}", inode.path.display(), e),
            ))
        })?;
        inode.raw_data = data;
    }

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
    use std::path::Path;

    use super::plan;
    use crate::testutil::test_config;

    #[test]
    fn invalid_source_errors() {
        // ARRANGE
        let nonexistent = Path::new("/nonexistent_dir_xyz");

        let result = plan(nonexistent, &test_config(1));

        // ACT
        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn readdir_order_nested_directories() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("a")).expect("mkdir");
        std::fs::write(dir.path().join("a").join("b"), b"x").expect("write");

        let plan = plan(dir.path(), &test_config(0)).expect("plan");

        // ACT
        // ASSERT
        assert!(plan.inodes.iter().any(|inode| inode.rel_path == "/"));
        assert!(plan.inodes.iter().any(|inode| inode.rel_path == "/a"));
        assert!(plan.inodes.iter().any(|inode| inode.rel_path == "/a/b"));
    }
}
