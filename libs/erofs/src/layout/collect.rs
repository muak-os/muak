//! Filesystem walking and initial inode construction from disk metadata.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use super::types::InodeLayout;
use crate::MkfsConfig;
use crate::dir::{self, EROFS_FT_DIR, EROFS_FT_REG_FILE, EROFS_FT_SYMLINK};
use crate::error::{ErofsError, Result};
use crate::inode::EROFS_INODE_FLAT_PLAIN;
use crate::xattr;

/// Normalize a relative path to a canonical form with leading `/`.
fn normalize_rel(path: &Path, prefix: &Path) -> String {
    let s = path
        .strip_prefix(prefix)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    if s.is_empty() {
        "/".to_string()
    } else {
        format!("/{s}")
    }
}

/// Walk the source directory and collect (absolute, relative) path pairs.
pub fn collect_entries(source_dir: &Path) -> Result<Vec<(PathBuf, String)>> {
    let walker = walkdir::WalkDir::new(source_dir)
        .sort_by_file_name()
        .follow_links(false);

    let mut entries = Vec::new();
    for entry in walker {
        let entry = entry?;
        let abs = entry.path().to_path_buf();
        let rel = normalize_rel(&abs, source_dir);
        entries.push((abs, rel));
    }
    Ok(entries)
}

/// Build initial `InodeLayout` entries from filesystem metadata.
pub fn build_initial_inodes(
    entries: &[(PathBuf, String)],
    config: &MkfsConfig<'_>,
) -> Result<Vec<InodeLayout>> {
    let mut inodes = Vec::with_capacity(entries.len());

    for (abs, rel) in entries {
        let meta = fs::symlink_metadata(abs).map_err(|e| {
            ErofsError::Io(std::io::Error::new(
                e.kind(),
                format!("{}: {}", abs.display(), e),
            ))
        })?;

        let name = if *rel == "/" {
            String::new()
        } else {
            abs.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        };

        if name.len() > dir::EROFS_NAME_LEN {
            return Err(ErofsError::FilenameTooLong(name));
        }

        let file_type = classify_file_type(&meta);
        let epoch = config.source_date_epoch;
        let (mtime, mtime_nsec) = resolve_mtime(&meta, epoch);

        let xattr_payload = config
            .file_contexts
            .and_then(|fc| fc.label_for(rel))
            .map(|l| xattr::build_selinux_xattr(l.as_bytes()))
            .unwrap_or_default();
        let xattr_ic = xattr::xattr_icount(xattr_payload.len());

        let symlink_target = read_symlink_target(abs, &meta)?;
        let idx = inodes.len();

        let size = if meta.is_dir() {
            0u32
        } else {
            meta.len() as u32
        };

        let uid = config.force_uid.unwrap_or(meta.uid() as u16);
        let gid = config.force_gid.unwrap_or(meta.gid() as u16);

        inodes.push(InodeLayout {
            path: abs.clone(),
            rel_path: rel.clone(),
            nid: 0,
            ino: idx as u32,
            mode: meta.mode() as u16,
            uid,
            gid,
            mtime,
            mtime_nsec,
            nlink: 1,
            file_type,
            size,
            datalayout: EROFS_INODE_FLAT_PLAIN,
            xattr_payload,
            xattr_icount: xattr_ic,
            inline_data: Vec::new(),
            data_blkaddr: 0,
            data_blocks: 0,
            children: Vec::new(),
            symlink_target,
            rdev: if meta.is_dir() || meta.is_file() || meta.is_symlink() {
                0
            } else {
                meta.rdev() as u32
            },
            compressed: None,
        });
    }
    Ok(inodes)
}

/// Classify a filesystem entry's type into an EROFS file type constant.
fn classify_file_type(meta: &fs::Metadata) -> u8 {
    if meta.is_dir() {
        EROFS_FT_DIR
    } else if meta.is_symlink() {
        EROFS_FT_SYMLINK
    } else {
        EROFS_FT_REG_FILE
    }
}

/// Resolve the modification time, using the source date epoch if configured.
fn resolve_mtime(meta: &fs::Metadata, epoch: u64) -> (u64, u32) {
    if epoch > 0 {
        (epoch, 0)
    } else {
        (meta.mtime() as u64, meta.mtime_nsec() as u32)
    }
}

/// Read the target of a symbolic link.
fn read_symlink_target(abs: &Path, meta: &fs::Metadata) -> Result<Vec<u8>> {
    if !meta.is_symlink() {
        return Ok(Vec::new());
    }
    let target = fs::read_link(abs).map_err(|_| ErofsError::SymlinkRead(abs.to_path_buf()))?;
    Ok(target.to_string_lossy().as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_rel_path_not_under_prefix() {
        // ARRANGE
        let path = Path::new("/other/path");
        let prefix = Path::new("/source");

        // ACT
        let result = normalize_rel(path, prefix);

        // ASSERT
        assert_eq!(result, "/");
    }

    #[test]
    fn collect_entries_reads_symlink_target() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::os::unix::fs::symlink("/target", dir.path().join("link")).expect("symlink");

        // ACT
        let entries = collect_entries(dir.path()).expect("entries");

        // ASSERT
        assert!(
            entries
                .iter()
                .any(|(abs, rel)| { abs.is_symlink() && *rel == "/link" })
        );
    }

    #[test]
    fn build_initial_inodes_force_uid_gid() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("f"), b"x").expect("write");
        let entries = collect_entries(dir.path()).expect("entries");
        let config = crate::MkfsConfig {
            source_date_epoch: 0,
            file_contexts: None,
            uuid: [0; 16],
            force_uid: Some(1234),
            force_gid: Some(5678),
            compress: false,
        };

        // ACT
        let inodes = build_initial_inodes(&entries, &config).expect("inodes");

        // ASSERT
        let file = inodes.iter().find(|i| i.rel_path == "/f").expect("found");
        assert_eq!(file.uid, 1234);
        assert_eq!(file.gid, 5678);
    }

    #[test]
    fn build_initial_inodes_uses_source_date_epoch() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("f"), b"x").expect("write");
        let entries = collect_entries(dir.path()).expect("entries");
        let config = crate::MkfsConfig {
            source_date_epoch: 1_700_000_000,
            file_contexts: None,
            uuid: [0; 16],
            force_uid: None,
            force_gid: None,
            compress: false,
        };

        // ACT
        let inodes = build_initial_inodes(&entries, &config).expect("inodes");

        // ASSERT: epoch overrides actual mtime.
        for inode in &inodes {
            assert_eq!(inode.mtime, 1_700_000_000);
            assert_eq!(inode.mtime_nsec, 0);
        }
    }

    #[test]
    fn build_initial_inodes_real_mtime_when_epoch_zero() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("f"), b"x").expect("write");
        let entries = collect_entries(dir.path()).expect("entries");
        let config = crate::MkfsConfig {
            source_date_epoch: 0,
            file_contexts: None,
            uuid: [0; 16],
            force_uid: None,
            force_gid: None,
            compress: false,
        };

        // ACT
        let inodes = build_initial_inodes(&entries, &config).expect("inodes");

        // ASSERT
        let file = inodes.iter().find(|i| i.rel_path == "/f").expect("found");
        assert!(file.mtime > 0);
    }

    #[test]
    fn build_initial_inodes_symlink_has_target() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::os::unix::fs::symlink("/some/target", dir.path().join("link")).expect("symlink");
        let entries = collect_entries(dir.path()).expect("entries");
        let config = crate::MkfsConfig {
            source_date_epoch: 0,
            file_contexts: None,
            uuid: [0; 16],
            force_uid: None,
            force_gid: None,
            compress: false,
        };

        // ACT
        let inodes = build_initial_inodes(&entries, &config).expect("inodes");

        // ASSERT
        let link = inodes
            .iter()
            .find(|i| i.rel_path == "/link")
            .expect("found");
        assert_eq!(link.symlink_target, b"/some/target");
    }
}
