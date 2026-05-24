//! Recursive source-tree collection helpers for ESP files.

use std::path::Path;

use crate::error::Result;
use crate::{EspError, EspFile};

/// Collects all regular files beneath `root` as ESP files.
///
/// # Errors
///
/// Returns an error when the tree cannot be read, contains symlinks, or contains paths
/// that cannot be converted into normalized UTF-8 ESP-relative paths.
pub fn collect_tree(root: &Path) -> Result<Vec<EspFile>> {
    let mut files = Vec::new();
    collect_dir(root, Path::new(""), &mut files)?;
    files.sort_unstable_by(compare_files_by_path);
    Ok(files)
}

/// Compares two ESP files by their paths for sorting.
fn compare_files_by_path(left: &EspFile, right: &EspFile) -> core::cmp::Ordering {
    left.path.cmp(&right.path)
}

/// Walks one source directory recursively and collects its files.
fn collect_dir(dir: &Path, rel_dir: &Path, files: &mut Vec<EspFile>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        let rel_path = rel_dir.join(entry.file_name());
        if file_type.is_dir() {
            collect_dir(&path, &rel_path, files)?;
            continue;
        }
        if file_type.is_symlink() {
            return Err(EspError::UnsupportedEntry(format!(
                "symlinks are not supported: {}",
                path.display()
            )));
        }

        let Some(rel_path) = rel_path.to_str() else {
            return Err(EspError::InvalidPath(format!(
                "non-UTF-8 source path: {}",
                path.display()
            )));
        };
        files.push(EspFile {
            path: rel_path.to_owned(),
            data: std::fs::read(&path)?,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;
    use std::path::Path;

    use super::collect_tree;
    use crate::EspError;

    #[test]
    fn collect_tree_recurses_and_preserves_relative_paths() {
        // ARRANGE
        let root = tempfile::tempdir().expect("temp dir must be created");
        std::fs::create_dir_all(root.path().join("firmware/boot")).expect("tree must be created");
        std::fs::write(root.path().join("firmware/boot/config.txt"), b"arm_64bit=1")
            .expect("config must be written");
        std::fs::write(root.path().join("start4.elf"), b"gpu-fw")
            .expect("firmware must be written");

        // ACT
        let files = collect_tree(root.path()).expect("collection must succeed");

        // ASSERT
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "firmware/boot/config.txt");
        assert_eq!(files[0].data, b"arm_64bit=1");
        assert_eq!(files[1].path, "start4.elf");
        assert_eq!(files[1].data, b"gpu-fw");
    }

    #[test]
    fn collect_tree_rejects_symlinks() {
        // ARRANGE
        let root = tempfile::tempdir().expect("temp dir must be created");
        std::fs::write(root.path().join("target.txt"), b"data").expect("target must be written");
        symlink(root.path().join("target.txt"), root.path().join("link.txt"))
            .expect("symlink must be created");

        // ACT
        let result = collect_tree(root.path());

        // ASSERT
        assert!(matches!(result, Err(EspError::UnsupportedEntry(_))));
    }

    #[test]
    fn collect_tree_ignores_empty_directories() {
        // ARRANGE
        let root = tempfile::tempdir().expect("temp dir must be created");
        std::fs::create_dir_all(root.path().join("boot/empty")).expect("tree must be created");

        // ACT
        let files = collect_tree(root.path()).expect("collection must succeed");

        // ASSERT
        assert!(files.is_empty());
    }

    #[test]
    fn collect_tree_fails_for_missing_root() {
        // ARRANGE / ACT
        let result = collect_tree(Path::new("/nonexistent/esp-tree"));

        // ASSERT
        assert!(matches!(result, Err(EspError::Io(_))));
    }
}
