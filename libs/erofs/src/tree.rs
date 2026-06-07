//! Source-tree abstractions for EROFS image building.

use crate::error::Result;

/// An entry in a source tree with all metadata pre-gathered.
#[derive(Debug, Clone)]
#[expect(
    clippy::module_name_repetitions,
    reason = "Tree prefix disambiguates from other types"
)]
pub struct TreeEntry {
    /// Relative path from the tree root (e.g. `/foo/bar`).
    pub rel_path: String,
    /// EROFS file type constant (`EROFS_FT_DIR`, `EROFS_FT_REG_FILE`, etc.).
    pub file_type: u8,
    /// File size in bytes.
    pub size: u64,
    /// File mode (`st_mode` bits).
    pub mode: u32,
    /// Owner UID.
    pub uid: u32,
    /// Owner GID.
    pub gid: u32,
    /// Modification time (seconds since epoch).
    pub mtime: u64,
    /// Nanosecond component of modification time.
    pub mtime_nsec: u32,
    /// Symlink target bytes (empty for non-symlinks).
    pub symlink_target: Vec<u8>,
    /// Device number for special/device files.
    pub rdev: u32,
}

/// Abstract source of a file tree.
///
/// Implementations enumerate entries deterministically and provide
/// file content on demand without exposing filesystem paths.
#[expect(
    clippy::module_name_repetitions,
    reason = "Tree prefix disambiguates from other sources"
)]
pub trait TreeSource {
    /// Enumerate all entries under the tree root.
    ///
    /// The returned list must be sorted by relative path in ascending
    /// lexicographic order and must be deterministic across calls.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying source cannot be read.
    fn entries(&self) -> Result<Vec<TreeEntry>>;

    /// Read the full content of an entry identified by its relative path.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry cannot be read or does not exist.
    fn read(&self, rel_path: &str) -> Result<Vec<u8>>;
}
