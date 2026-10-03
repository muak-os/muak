//! Data types for pulled image entries.

use std::io::Read;

/// A streamable file entry from an OCI image.
pub struct FileEntry<'a> {
    /// Path of the file relative to the image root.
    pub path: String,
    /// File size in bytes.
    pub size: u64,
    /// UNIX file mode bits.
    pub mode: u32,
    /// Readable stream for file data.
    pub reader: &'a mut dyn Read,
}
