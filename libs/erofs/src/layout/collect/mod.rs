//! Filesystem walking and initial inode construction from disk metadata.

mod inode_builder;
mod walk;

use std::path::{Path, PathBuf};

use crate::MkfsConfig;
use crate::error::Result;
use crate::layout::types::InodeLayout;

/// Walk the source directory recursively and collect sorted absolute and relative paths.
pub fn entries(source_dir: &Path) -> Result<Vec<(PathBuf, String)>> {
    walk::entries(source_dir)
}

/// Build initial `InodeLayout` entries from filesystem metadata.
pub fn initial_inodes(
    entries: &[(PathBuf, String)],
    config: &MkfsConfig<'_>,
) -> Result<Vec<InodeLayout>> {
    inode_builder::initial_inodes(entries, config)
}
