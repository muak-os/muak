//! Initial inode construction from tree entries.

mod inode_builder;

use crate::MkfsConfig;
use crate::error::Result;
use crate::layout::Inode;
use crate::source::TreeEntry;

/// Build initial `Inode` entries from tree entries.
///
/// # Errors
///
/// Returns an error when an entry filename exceeds the EROFS name length limit.
pub(crate) fn initial_inodes(entries: &[TreeEntry], config: &MkfsConfig<'_>) -> Result<Vec<Inode>> {
    inode_builder::initial_inodes(entries, config)
}
