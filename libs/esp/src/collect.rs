//! Recursive source-tree collection helpers for ESP files.

use std::path::Path;

use crate::{EspError, EspFile};

/// Collects all regular files beneath `root` as ESP files.
pub fn collect_tree(root: &Path) -> Result<Vec<EspFile>, EspError> {
    let mut files = Vec::new();
    collect_dir(root, root, &mut files)?;
    files.sort_unstable_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

/// Walks one source directory recursively and collects its files.
fn collect_dir(root: &Path, dir: &Path, files: &mut Vec<EspFile>) -> Result<(), EspError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_dir(root, &path, files)?;
            continue;
        }
        if file_type.is_symlink() {
            return Err(EspError::UnsupportedEntry(format!(
                "symlinks are not supported: {}",
                path.display()
            )));
        }
        if !file_type.is_file() {
            continue;
        }

        let rel_path = path.strip_prefix(root).map_err(|_| {
            EspError::InvalidPath(format!("failed to strip root: {}", path.display()))
        })?;
        let rel_path = rel_path.to_str().ok_or_else(|| {
            EspError::InvalidPath(format!("non-UTF-8 source path: {}", path.display()))
        })?;
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
