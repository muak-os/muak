use std::io::Read;
use std::path::Path;

use anyhow::{Context as _, Result};
use erofs::dir::EROFS_FT_REG_FILE;
use erofs::tree::TreeEntry;

pub const REQUIRED_DIRS: &[&str] = &["dev", "proc", "sys", "run", "etc/services", "etc/selinux"];

/// Creates required Linux boot directories under `root` if they don't exist.
///
/// # Errors
///
/// Returns an error if a directory cannot be created.
pub fn inject_required_dirs(root: &Path) -> Result<()> {
    for dir in REQUIRED_DIRS {
        std::fs::create_dir_all(root.join(dir))
            .with_context(|| format!("Failed to create required directory: {dir}"))?;
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
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create parent for {}", path.display()))?;
    }
    std::os::unix::fs::symlink("/run/resolv.conf", path)
        .with_context(|| format!("Failed to create symlink at {}", path.display()))?;

    Ok(())
}

#[derive(Debug)]
pub enum EntryReader {
    File(std::fs::File),
    Empty(std::io::Empty),
}

impl Read for EntryReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match *self {
            Self::File(ref mut file) => file.read(buf),
            Self::Empty(ref mut empty) => empty.read(buf),
        }
    }
}

/// Opens file readers for each entry, using an empty reader for directories and symlinks.
///
/// # Errors
///
/// Returns an error if a regular file with non-zero size cannot be opened.
pub fn build_readers(dir: &Path, entries: &[TreeEntry]) -> Result<Vec<EntryReader>> {
    entries
        .iter()
        .map(|ent| {
            if ent.file_type == EROFS_FT_REG_FILE && ent.size > 0 {
                let path = dir.join(ent.rel_path.strip_prefix('/').unwrap_or(&ent.rel_path));
                match std::fs::File::open(&path) {
                    Ok(file) => Ok(EntryReader::File(file)),
                    Err(source) => Err(anyhow::anyhow!(
                        "Failed to open {}: {source}",
                        path.display()
                    )),
                }
            } else {
                Ok(EntryReader::Empty(std::io::empty()))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

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
        let reader = readers.first_mut().unwrap();
        let mut buf = String::new();
        reader.read_to_string(&mut buf).unwrap();
        assert_eq!(buf, "hello");
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
        let reader = readers.first_mut().unwrap();
        let mut buf = Vec::new();
        let n = reader.read_to_end(&mut buf).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn build_readers_directory_gives_empty_reader() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let entries = vec![tree_entry("/", 1, 0)];

        // ACT
        let mut readers = build_readers(dir.path(), &entries).unwrap();

        // ASSERT
        let reader = readers.first_mut().unwrap();
        let mut buf = Vec::new();
        let n = reader.read_to_end(&mut buf).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn build_readers_missing_file() {
        // ARRANGE
        let entries = vec![tree_entry("/missing", EROFS_FT_REG_FILE, 5)];

        // ACT / ASSERT
        build_readers(Path::new("/tmp"), &entries).unwrap_err();
    }

    #[test]
    fn entry_reader_file_read() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f"), b"data").unwrap();
        let file = std::fs::File::open(dir.path().join("f")).unwrap();
        let mut reader = EntryReader::File(file);

        // ACT
        let mut buf = [0_u8; 4];
        let n = reader.read(&mut buf).unwrap();

        // ASSERT
        assert_eq!(n, 4);
        assert_eq!(&buf, b"data");
    }

    #[test]
    fn entry_reader_empty_read() {
        // ARRANGE
        let mut reader = EntryReader::Empty(std::io::empty());

        // ACT
        let mut buf = [0_u8; 4];
        let n = reader.read(&mut buf).unwrap();

        // ASSERT
        assert_eq!(n, 0);
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
        assert_eq!(readers.len(), 2);
        let dir_reader = readers.first_mut().unwrap();
        let mut buf = Vec::new();
        dir_reader.read_to_end(&mut buf).unwrap();
        assert_eq!(buf.len(), 0); // dir = empty
        let file_reader = readers.get_mut(1).unwrap();
        let mut buf = Vec::new();
        file_reader.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, b"x");
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
