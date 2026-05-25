//! Layout planning for inode metadata and data blocks.

mod assign;
mod collect;
mod indices;
mod planner;
mod types;

use std::path::Path;

pub(crate) use assign::index_layout;

/// Compute the total image size from the planned layout.
pub fn total_image_size(inodes: &[types::InodeLayout], do_compress: bool) -> usize {
    assign::total_image_size(inodes, do_compress)
}

/// Public inode layout type produced by layout planning.
pub type InodeLayout = types::InodeLayout;

use crate::MkfsConfig;
use crate::error::Result;

/// Plan the full image layout from a source directory.
pub fn plan(source_dir: &Path, config: &MkfsConfig<'_>) -> Result<Vec<InodeLayout>> {
    planner::plan(source_dir, config)
}

/// Compute parent relative path from a child relative path.
pub(super) fn parent_rel(rel: &str) -> String {
    if rel == "/" {
        return "/".to_owned();
    }
    let parent = Path::new(rel)
        .parent()
        .unwrap_or(Path::new("/"))
        .to_string_lossy()
        .to_string();
    if parent.is_empty() {
        "/".to_owned()
    } else {
        parent
    }
}

#[cfg(test)]
mod tests {
    use super::parent_rel;

    #[test]
    fn parent_rel_root_is_root() {
        // ARRANGE
        // ACT
        // ASSERT
        assert_eq!(parent_rel("/"), "/");
    }

    #[test]
    fn parent_rel_nested_path() {
        // ARRANGE
        // ACT
        // ASSERT
        assert_eq!(parent_rel("/a"), "/");
        assert_eq!(parent_rel("/a/b"), "/a");
        assert_eq!(parent_rel("/a/b/c"), "/a/b");
    }
}
