//! In-memory representation of a single inode's planned on-disk layout.

use crate::compress::CompressedFile;

/// Planned layout for a single inode.
#[derive(Debug, Clone)]
pub struct InodeLayout {
    pub rel_path: String,
    pub nid: u64,
    pub ino: u32,
    pub mode: u16,
    pub uid: u16,
    pub gid: u16,
    pub mtime: u64,
    pub mtime_nsec: u32,
    pub nlink: u16,
    pub file_type: u8,
    pub size: u32,
    pub datalayout: u16,
    pub xattr_payload: Vec<u8>,
    pub xattr_icount: u16,
    pub inline_data: Vec<u8>,
    pub raw_data: Vec<u8>,
    pub data_blkaddr: u32,
    pub data_blocks: u32,
    pub children: Vec<String>,
    pub symlink_target: Vec<u8>,
    pub rdev: u32,
    pub compressed: Option<CompressedFile>,
}
