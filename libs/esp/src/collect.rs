//! Recursive source-tree collection helpers for ESP files.

use std::path::Path;

use crate::error::{EspError, Result};
use crate::model::EspFile;

/// Walks `root` recursively, returning a streaming [`EspFile`] per regular file; symlinks are rejected.
///
/// # Errors
///
/// Returns an error when the tree cannot be read, contains symlinks, or contains paths that cannot be converted into normalized UTF-8 ESP-relative paths.
pub fn tree(root: &Path) -> Result<Vec<EspFile>> {
    let mut files = Vec::new();
    collect_dir(root, Path::new(""), &mut files)?;
    files.sort_unstable_by(|left, right| left.path.cmp(&right.path));

    Ok(files)
}

/// Walks `root` recursively, appending streaming [`EspFile`]s into `out`; symlinks are rejected.
///
/// # Errors
///
/// Returns an error when the tree cannot be read, contains symlinks, or contains paths that cannot be converted into normalized UTF-8 ESP-relative paths.
pub fn into(root: &Path, out: &mut Vec<EspFile>) -> Result<()> {
    collect_dir(root, Path::new(""), out)?;
    out.sort_unstable_by(|left, right| left.path.cmp(&right.path));

    Ok(())
}

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
        let file = std::fs::File::open(&path)?;
        let size = file.metadata()?.len();
        files.push(EspFile {
            path: rel_path.to_owned(),
            reader: Box::new(file),
            size,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;
    use std::path::Path;

    use super::tree;
    use crate::error::EspError;
    use crate::model::EspFile;

    fn read_into(file: &mut EspFile) -> Vec<u8> {
        use std::io::Read as _;
        let mut buf = Vec::new();
        file.reader
            .read_to_end(&mut buf)
            .expect("read must succeed");

        buf
    }

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
        let mut files = tree(root.path()).expect("collection must succeed");

        // ASSERT
        assert_eq!(files.len(), 2);
        let (config_path, config_data) = {
            let config = files.first_mut().expect("first file must exist");
            (config.path.clone(), read_into(config))
        };
        let firmware = files.get_mut(1).expect("second file must exist");
        assert_eq!(config_path, "firmware/boot/config.txt");
        assert_eq!(config_data, b"arm_64bit=1");
        assert_eq!(firmware.path, "start4.elf");
        assert_eq!(read_into(firmware), b"gpu-fw");
    }

    #[test]
    fn collect_tree_rejects_symlinks() {
        // ARRANGE
        let root = tempfile::tempdir().expect("temp dir must be created");
        std::fs::write(root.path().join("target.txt"), b"data").expect("target must be written");
        symlink(root.path().join("target.txt"), root.path().join("link.txt"))
            .expect("symlink must be created");

        // ACT
        let result = tree(root.path());

        // ASSERT
        assert!(matches!(result, Err(EspError::UnsupportedEntry(_))));
    }

    #[test]
    fn collect_tree_ignores_empty_directories() {
        // ARRANGE
        let root = tempfile::tempdir().expect("temp dir must be created");
        std::fs::create_dir_all(root.path().join("boot/empty")).expect("tree must be created");

        // ACT
        let files = tree(root.path()).expect("collection must succeed");

        // ASSERT
        assert!(files.is_empty());
    }

    #[test]
    fn collect_tree_fails_for_missing_root() {
        // ARRANGE / ACT
        let result = tree(Path::new("/nonexistent/esp-tree"));

        // ASSERT
        assert!(matches!(result, Err(EspError::Io(_))));
    }
}
