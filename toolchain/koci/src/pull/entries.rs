//! Data types for pulled image entries.

use std::io::Read;

/// The filesystem object an image entry describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A regular file whose bytes stream through [`FileEntry::reader`].
    File,
    /// A symbolic link whose target is [`FileEntry::link`].
    Symlink,
    /// A hard link whose image-relative target is [`FileEntry::link`].
    HardLink,
}

/// POSIX metadata carried by an image entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    /// UNIX permission bits.
    pub mode: u32,
    /// Owner user id.
    pub uid: u32,
    /// Owner group id.
    pub gid: u32,
    /// Modification time in seconds since the Unix epoch.
    pub mtime: u64,
}

/// A streamable entry from an OCI image.
pub struct FileEntry<'a> {
    /// Path of the entry relative to the image root.
    pub path: String,
    /// Filesystem object kind.
    pub kind: Kind,
    /// Size in bytes of the streamed data, zero for links.
    pub size: u64,
    /// Symlink target bytes, or the image-relative hard link target path.
    pub link: Option<String>,
    /// POSIX metadata of the entry.
    pub meta: Meta,
    /// Readable stream for file data, empty for links.
    pub reader: &'a mut dyn Read,
}
