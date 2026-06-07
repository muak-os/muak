//! Source-tree abstraction and filesystem-backed implementation.

mod inode_builder;
mod walk;

use std::path::Path;

use crate::MkfsConfig;
use crate::error::{ErofsError, Result};
use crate::layout::types::InodeLayout;
use crate::tree::{TreeEntry, TreeSource};

/// Build initial `InodeLayout` entries from tree entries.
///
/// # Errors
///
/// Returns an error when an entry filename exceeds the EROFS name length limit.
pub fn initial_inodes(entries: &[TreeEntry], config: &MkfsConfig<'_>) -> Result<Vec<InodeLayout>> {
    inode_builder::initial_inodes(entries, config)
}

/// A [`TreeSource`] backed by a real filesystem directory.
pub struct FilesystemTreeSource<'a> {
    root: &'a Path,
}

impl<'a> FilesystemTreeSource<'a> {
    pub fn new(root: &'a Path) -> Self {
        Self { root }
    }
}

impl TreeSource for FilesystemTreeSource<'_> {
    fn entries(&self) -> Result<Vec<TreeEntry>> {
        let raw = walk::entries(self.root)?;
        let mut entries = Vec::with_capacity(raw.len());
        for abs_rel in &raw {
            let (abs, rel) = (abs_rel.0.as_path(), abs_rel.1.as_str());
            let meta = walk::symlink_metadata_with_context(abs)
                .map_err(|e| ErofsError::Walk(format!("{}: {e}", abs.display())))?;
            entries.push(inode_builder::entry_from_meta(abs, rel, &meta)?);
        }
        Ok(entries)
    }

    fn read(&self, rel_path: &str) -> Result<Vec<u8>> {
        let full_path = if rel_path == "/" {
            self.root.to_path_buf()
        } else {
            let stripped = rel_path.strip_prefix('/').unwrap_or(rel_path);
            self.root.join(stripped)
        };
        std::fs::read(&full_path).map_err(|e| {
            ErofsError::Io(std::io::Error::new(
                e.kind(),
                format!("{}: {e}", full_path.display()),
            ))
        })
    }
}
