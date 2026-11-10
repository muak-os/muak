//! Sized file entry type and filesystem directory walking.

use std::io::Read;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::{fs, io};

use crate::dir::{EROFS_FT_DIR, EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
use crate::error::{ErofsError, Result};
/// An entry in a source tree with all metadata pre-gathered.
#[derive(Debug, Clone)]
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

/// A file entry with metadata and a readable stream.
pub struct SizedFile<'a> {
    /// File metadata (path, type, size, permissions, etc.).
    pub entry: TreeEntry,
    /// Reader for file data.
    pub reader: &'a mut dyn Read,
}

/// Walk a filesystem directory and collect metadata entries.
///
/// # Errors
///
/// Returns an error when the source directory does not exist, is not a directory,
/// or entries within it cannot be read.
pub fn collect_entries(source_dir: &Path) -> Result<Vec<TreeEntry>> {
    if !source_dir.is_dir() {
        return Err(ErofsError::InvalidSource(source_dir.to_path_buf()));
    }
    let mut raw = vec![(source_dir.to_path_buf(), "/".to_owned())];
    recurse(source_dir, source_dir, &mut raw)?;
    raw.sort_unstable_by(|lhs, rhs| lhs.1.cmp(&rhs.1));

    raw.iter()
        .map(|entry| -> Result<TreeEntry> {
            let abs = &entry.0;
            let rel = &entry.1;
            let meta = fs::symlink_metadata(abs).map_err(|err| {
                ErofsError::Io(io::Error::new(
                    err.kind(),
                    format!("{}: {err}", abs.display()),
                ))
            })?;
            entry_from_meta(abs, rel, &meta)
        })
        .collect()
}

fn recurse(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, String)>) -> Result<()> {
    let read_dir =
        fs::read_dir(dir).map_err(|err| ErofsError::Walk(format!("{}: {err}", dir.display())))?;

    let mut names: Vec<_> = read_dir
        .map(|entry| entry.map(|e| e.file_name()))
        .collect::<core::result::Result<_, _>>()
        .map_err(|err| ErofsError::Walk(format!("{}: {err}", dir.display())))?;
    names.sort_unstable();

    for name in names {
        let abs = dir.join(&name);
        let rel = normalize_rel(&abs, root);
        let meta = fs::symlink_metadata(&abs)
            .map_err(|err| ErofsError::Walk(format!("{}: {err}", abs.display())))?;
        out.push((abs.clone(), rel));
        if meta.is_dir() {
            recurse(root, &abs, out)?;
        }
    }
    Ok(())
}

fn normalize_rel(path: &Path, prefix: &Path) -> String {
    let relative = path
        .strip_prefix(prefix)
        .map_or_default(|pref| pref.to_string_lossy().to_string());
    if relative.is_empty() {
        "/".to_owned()
    } else {
        format!("/{relative}")
    }
}

fn entry_from_meta(abs: &Path, rel: &str, meta: &fs::Metadata) -> Result<TreeEntry> {
    let symlink_target = if meta.is_symlink() {
        fs::read_link(abs)
            .map_err(|_err| ErofsError::SymlinkRead(abs.to_path_buf()))?
            .to_string_lossy()
            .as_bytes()
            .to_vec()
    } else {
        Vec::new()
    };

    let file_type = classify_file_type(meta);

    Ok(TreeEntry {
        rel_path: rel.to_owned(),
        file_type,
        size: meta.len(),
        mode: meta.mode(),
        uid: meta.uid(),
        gid: meta.gid(),
        mtime: meta.mtime().cast_unsigned(),
        mtime_nsec: u32::try_from(meta.mtime_nsec()).unwrap_or_default(),
        symlink_target,
        rdev: if meta.is_dir() || meta.is_file() || meta.is_symlink() {
            0
        } else {
            u32::try_from(meta.rdev() & 0xFFFF_FFFF).unwrap_or_default()
        },
    })
}

fn classify_file_type(meta: &fs::Metadata) -> u8 {
    if meta.is_dir() {
        EROFS_FT_DIR
    } else if meta.is_symlink() {
        EROFS_FT_SYMLINK
    } else {
        EROFS_FT_REG_FILE
    }
}
