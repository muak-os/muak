//! Shared fixtures for EROFS unit and integration tests.

use std::io::Read;

use erofs::MkfsConfig;
use erofs::compress::Compression;
use erofs::layout::Inode;
use erofs::source::{SizedFile, TreeEntry};

/// Cursor set over shared content (test support).
pub type CursorSet<'a> = Vec<std::io::Cursor<&'a [u8]>>;

/// Returns a minimal uncompressed [`MkfsConfig`] for use in tests.
#[must_use]
pub(crate) fn test_config(epoch: u64) -> MkfsConfig<'static> {
    MkfsConfig {
        source_date_epoch: epoch,
        file_contexts: None,
        uuid: [0; 16],
        force_uid: None,
        force_gid: None,
        compression: Compression::None,
    }
}

/// Returns a minimal compressed [`MkfsConfig`] for use in tests.
#[must_use]
pub(crate) fn compress_config(epoch: u64) -> MkfsConfig<'static> {
    MkfsConfig {
        source_date_epoch: epoch,
        file_contexts: None,
        uuid: [0; 16],
        force_uid: None,
        force_gid: None,
        compression: Compression::default(),
    }
}

/// Reconstructs a [`TreeEntry`] from a planned inode (test support).
#[must_use]
pub fn entry_of(inode: &Inode) -> TreeEntry {
    TreeEntry {
        rel_path: inode.rel_path.clone(),
        file_type: inode.file_type,
        size: u64::from(inode.size),
        mode: u32::from(inode.mode),
        uid: u32::from(inode.uid),
        gid: u32::from(inode.gid),
        mtime: inode.mtime,
        mtime_nsec: inode.mtime_nsec,
        symlink_target: inode.symlink_target.clone(),
        rdev: inode.rdev,
    }
}

/// Zero-filled placeholder content matching each entry's size (test support).
///
/// # Panics
///
/// Panics when an entry size cannot be represented as `usize`.
#[must_use]
pub fn zero_data(entry: &TreeEntry) -> Vec<u8> {
    if entry.file_type == erofs::dir::EROFS_FT_REG_FILE && entry.size > 0 {
        vec![0_u8; usize::try_from(entry.size).expect("size fits usize")]
    } else {
        Vec::new()
    }
}

/// Pairs owned entries with mutable readers into [`SizedFile`]s (test support).
pub fn pair_files<R: Read>(entries: Vec<TreeEntry>, readers: &mut [R]) -> Vec<SizedFile<'_>> {
    entries
        .into_iter()
        .zip(readers.iter_mut())
        .map(|(entry, reader)| SizedFile { entry, reader })
        .collect()
}

/// Builds one independent cursor set over shared content (test support).
#[must_use]
pub fn cursor_set(data: &[Vec<u8>]) -> CursorSet<'_> {
    data.iter()
        .map(|bytes| std::io::Cursor::new(bytes.as_slice()))
        .collect()
}
