//! Rootfs-specific directory injection and per-entry file readers.

use std::io::{self, Read};
use std::path::Path;

use erofs::dir::EROFS_FT_REG_FILE;
use erofs::tree::TreeEntry;

use crate::error::{MumiError, Result};

/// Required Linux boot directories.
pub const REQUIRED_DIRS: &[&str] = &["dev", "proc", "sys", "run", "etc/services", "etc/selinux"];

/// Creates required Linux boot directories under `root` if they don't exist.
///
/// # Errors
///
/// Returns an error if a directory cannot be created.
pub fn inject_required_dirs(root: &Path) -> Result<()> {
    for dir in REQUIRED_DIRS {
        std::fs::create_dir_all(root.join(dir)).map_err(|source| {
            MumiError::Io(io::Error::other(format!(
                "Failed to create required directory: {dir}: {source}"
            )))
        })?;
    }

    Ok(())
}

/// Creates `/etc/resolv.conf` as a symlink to `/run/resolv.conf` if it doesn't exist.
///
/// # Errors
///
/// Returns an error if the parent directory cannot be created or the symlink fails.
pub fn ensure_default_resolv_conf(path: &Path) -> Result<()> {
    if std::fs::symlink_metadata(path).is_ok() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| {
            MumiError::Io(io::Error::other(format!(
                "Failed to create parent for {}: {source}",
                path.display()
            )))
        })?;
    }
    std::os::unix::fs::symlink("/run/resolv.conf", path).map_err(|source| {
        MumiError::Io(io::Error::other(format!(
            "Failed to create symlink at {}: {source}",
            path.display()
        )))
    })?;

    Ok(())
}

/// One sequential `Read` view per entry, for a single consumer pass.
#[derive(Debug)]
pub struct FileReaders {
    files: Vec<Option<std::fs::File>>,
    empties: Vec<std::io::Empty>,
}

impl FileReaders {
    /// Borrows every entry as a `Read` view.
    pub fn views(&mut self) -> Vec<&mut dyn Read> {
        self.files
            .iter_mut()
            .zip(self.empties.iter_mut())
            .map(|(file, empty)| -> &mut dyn Read {
                match file.as_mut() {
                    Some(file) => file,
                    None => empty,
                }
            })
            .collect()
    }
}

/// Opens file readers for each entry, using an empty reader for directories and symlinks.
///
/// # Errors
///
/// Returns an error if a regular file with non-zero size cannot be opened.
pub fn build_readers(dir: &Path, entries: &[TreeEntry]) -> Result<FileReaders> {
    let mut files = Vec::with_capacity(entries.len());
    for ent in entries {
        let should_open = ent.file_type == EROFS_FT_REG_FILE && ent.size > 0;
        if !should_open {
            files.push(None);
            continue;
        }
        let path = dir.join(ent.rel_path.strip_prefix('/').unwrap_or(&ent.rel_path));
        let file = std::fs::File::open(&path).map_err(|source| {
            MumiError::Io(io::Error::other(format!(
                "Failed to open {}: {source}",
                path.display()
            )))
        })?;
        files.push(Some(file));
    }
    let empties = vec![io::empty(); entries.len()];

    Ok(FileReaders { files, empties })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree_entry(rel_path: &str, file_type: u8, size: u64) -> TreeEntry {
        TreeEntry {
            rel_path: rel_path.to_owned(),
            file_type,
            size,
            mode: 0,
            uid: 0,
            gid: 0,
            mtime: 0,
            mtime_nsec: 0,
            symlink_target: vec![],
            rdev: 0,
        }
    }

    fn read_all(readers: &mut FileReaders, index: usize) -> Vec<u8> {
        let mut views = readers.views();
        let mut buf = Vec::new();
        let mut chunk = [0_u8; 64];
        let view: &mut dyn Read = &mut **views.get_mut(index).expect("entry view");
        while let Ok(n) = view.read(&mut chunk)
            && n > 0
        {
            buf.extend_from_slice(chunk.get(..n).unwrap_or_default());
        }

        buf
    }

    #[test]
    fn inject_required_dirs_all_created() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();

        // ACT
        inject_required_dirs(dir.path()).unwrap();

        // ASSERT
        for dir_name in REQUIRED_DIRS {
            assert!(dir.path().join(dir_name).is_dir(), "missing: {dir_name}");
        }
    }

    #[test]
    fn inject_required_dirs_already_exist() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        for dir_name in REQUIRED_DIRS {
            std::fs::create_dir_all(dir.path().join(dir_name)).unwrap();
        }

        // ACT
        inject_required_dirs(dir.path()).unwrap();

        // ASSERT — no error
    }

    #[test]
    fn ensure_resolv_conf_already_exists() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let resolv = dir.path().join("etc/resolv.conf");
        std::fs::create_dir_all(resolv.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink("/usr/lib/resolv.conf", &resolv).unwrap();

        // ACT
        ensure_default_resolv_conf(&resolv).unwrap();

        // ASSERT
        let target = std::fs::read_link(&resolv).unwrap();
        assert_eq!(target, std::path::Path::new("/usr/lib/resolv.conf"));
    }

    #[test]
    fn ensure_resolv_conf_creates_parent() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let resolv = dir.path().join("deeply/nested/etc/resolv.conf");

        // ACT
        ensure_default_resolv_conf(&resolv).unwrap();

        // ASSERT
        resolv.symlink_metadata().unwrap();
        let target = std::fs::read_link(&resolv).unwrap();
        assert_eq!(target, std::path::Path::new("/run/resolv.conf"));
    }

    #[test]
    fn build_readers_regular_file() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f"), b"hello").unwrap();
        let entries = vec![tree_entry("/f", EROFS_FT_REG_FILE, 5)];

        // ACT
        let mut readers = build_readers(dir.path(), &entries).unwrap();

        // ASSERT
        assert_eq!(read_all(&mut readers, 0), b"hello");
    }

    #[test]
    fn build_readers_empty_file_gives_empty_reader() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("e"), b"").unwrap();
        let entries = vec![tree_entry("/e", EROFS_FT_REG_FILE, 0)];

        // ACT
        let mut readers = build_readers(dir.path(), &entries).unwrap();

        // ASSERT
        assert!(read_all(&mut readers, 0).is_empty());
    }

    #[test]
    fn build_readers_directory_gives_empty_reader() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let entries = vec![tree_entry("/", 1, 0)];

        // ACT
        let mut readers = build_readers(dir.path(), &entries).unwrap();

        // ASSERT
        assert!(read_all(&mut readers, 0).is_empty());
    }

    #[test]
    fn build_readers_missing_file() {
        // ARRANGE
        let entries = vec![tree_entry("/missing", EROFS_FT_REG_FILE, 5)];

        // ACT / ASSERT
        build_readers(Path::new("/tmp"), &entries).unwrap_err();
    }

    #[test]
    fn build_readers_mixed_entry_types() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/f"), b"x").unwrap();
        let entries = vec![
            tree_entry("/sub", 1, 0),
            tree_entry("/sub/f", EROFS_FT_REG_FILE, 1),
        ];

        // ACT
        let mut readers = build_readers(dir.path(), &entries).unwrap();

        // ASSERT
        assert!(read_all(&mut readers, 0).is_empty());
        assert_eq!(read_all(&mut readers, 1), b"x");
    }

    #[test]
    fn inject_required_dirs_file_instead_of_dir() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("dev"), b"not-a-dir").unwrap();

        // ACT
        let result = inject_required_dirs(dir.path());

        // ASSERT
        result.unwrap_err();
    }
}
