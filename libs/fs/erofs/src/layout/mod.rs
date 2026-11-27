//! Layout planning for inode metadata and data blocks.

mod assign;
pub(crate) mod collect;
mod indices;
mod planner;

use std::path::Path;

pub(crate) use assign::index_layout;

use crate::MkfsConfig;
use crate::compress::CompressedLayout;
use crate::error::Result;
use crate::source::SizedFile;

/// Planned layout for a single inode.
#[derive(Debug, Clone)]
pub struct Inode {
    /// Path of the inode relative to the image root.
    pub rel_path: String,
    /// EROFS inode number (nid) assigned to this inode.
    pub nid: u64,
    /// POSIX inode number exposed through stat.
    pub ino: u32,
    /// POSIX mode bits (file type and permissions).
    pub mode: u16,
    /// Owning user ID.
    pub uid: u16,
    /// Owning group ID.
    pub gid: u16,
    /// Modification time in seconds since the epoch.
    pub mtime: u64,
    /// Nanosecond part of the modification time.
    pub mtime_nsec: u32,
    /// Number of hard links referencing this inode.
    pub nlink: u16,
    /// EROFS directory entry file type.
    pub file_type: u8,
    /// Logical file size in bytes.
    pub size: u32,
    /// EROFS data layout selector for the inode.
    pub datalayout: u16,
    /// Encoded extended-attribute payload.
    pub xattr_payload: Vec<u8>,
    /// Packed `i_xattr_icount`: 4-byte units occupied by the inline xattr area.
    pub xattr_icount: u16,
    /// Absolute block address of the inode's data.
    pub data_blkaddr: u32,
    /// Number of physical blocks backing the inode's data.
    pub data_blocks: u32,
    /// Relative paths of a directory's children.
    pub children: Vec<String>,
    /// Target path bytes for a symbolic link.
    pub symlink_target: Vec<u8>,
    /// Device number for special files.
    pub rdev: u32,
    /// Compressed layout metadata when the file is compressed.
    pub compressed: Option<CompressedLayout>,
}

/// A fully-planned EROFS image, ready for emission.
#[derive(Debug)]
pub struct ImagePlan {
    /// Planned inodes with complete layout metadata.
    pub inodes: Vec<Inode>,
    /// Total image size in bytes (block-aligned).
    pub total_size: usize,
    /// Whether compression is enabled for this image.
    pub do_compress: bool,
}

/// Plan the full image layout from sized file entries.
///
/// # Errors
///
/// Returns an error when entries are invalid, file data cannot be read or compression fails.
pub fn plan(files: &mut [SizedFile<'_>], config: &MkfsConfig<'_>) -> Result<ImagePlan> {
    planner::plan(files, config)
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

pub(crate) fn compute_meta_end(inodes: &[Inode], do_compress: bool) -> usize {
    inodes
        .iter()
        .map(|inode| {
            assign::sizes::nid_slot_offset(inode.nid)
                .saturating_add(assign::sizes::meta_size_bytes(inode))
        })
        .max()
        .unwrap_or(assign::meta_start(do_compress))
}

#[cfg(test)]
mod tests {
    use super::parent_rel;

    #[test]
    fn parent_rel_root_is_root() {
        // ARRANGE & ACT & ASSERT
        assert_eq!(parent_rel("/"), "/");
    }

    #[test]
    fn parent_rel_nested_path() {
        // ARRANGE & ACT & ASSERT
        assert_eq!(parent_rel("/a"), "/");
        assert_eq!(parent_rel("/a/b"), "/a");
        assert_eq!(parent_rel("/a/b/c"), "/a/b");
    }
}
