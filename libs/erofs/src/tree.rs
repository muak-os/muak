//! Source-tree entry type for EROFS image building.

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
