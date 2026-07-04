//! Initial inode construction from tree entries.

mod inode_builder;

use crate::MkfsConfig;
use crate::error::Result;
use crate::layout::types::InodeLayout;
use crate::tree::TreeEntry;

/// Build initial `InodeLayout` entries from tree entries.
///
/// # Errors
///
/// Returns an error when an entry filename exceeds the EROFS name length limit.
pub fn initial_inodes(entries: &[TreeEntry], config: &MkfsConfig<'_>) -> Result<Vec<InodeLayout>> {
    inode_builder::initial_inodes(entries, config)
}
