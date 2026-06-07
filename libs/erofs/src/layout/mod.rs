//! Layout planning for inode metadata and data blocks.

mod assign;
pub(crate) mod collect;
mod indices;
mod planner;
mod types;

use std::path::Path;

pub(crate) use assign::index_layout;

use crate::MkfsConfig;
use crate::error::Result;
use crate::tree::TreeSource;

/// Public inode layout type produced by layout planning.
pub type InodeLayout = types::InodeLayout;

/// A fully-planned EROFS image, ready for emission.
#[derive(Debug)]
pub struct ImagePlan {
    /// Planned inodes with complete layout metadata.
    pub inodes: Vec<InodeLayout>,
    /// Total image size in bytes (block-aligned).
    pub total_size: usize,
    /// Whether compression is enabled for this image.
    pub do_compress: bool,
}

/// Plan the full image layout from a source tree.
pub fn plan(source: &dyn TreeSource, config: &MkfsConfig<'_>) -> Result<ImagePlan> {
    planner::plan(source, config)
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
