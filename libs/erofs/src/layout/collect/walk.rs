//! Source-tree walking and relative-path normalization.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{ErofsError, Result};

/// Normalize a relative path to a canonical form with leading `/`.
pub(super) fn normalize_rel(path: &Path, prefix: &Path) -> String {
    let relative = path
        .strip_prefix(prefix)
        .map(|relative_path| relative_path.to_string_lossy().to_string())
        .unwrap_or_default();
    if relative.is_empty() {
        "/".to_owned()
    } else {
        format!("/{relative}")
    }
}

/// Walk the source directory recursively and collect sorted absolute and relative paths.
pub fn entries(source_dir: &Path) -> Result<Vec<(PathBuf, String)>> {
    let mut entries = vec![(source_dir.to_path_buf(), "/".to_owned())];
    recurse(source_dir, source_dir, &mut entries)?;
    entries.sort_unstable_by(|left, right| left.1.cmp(&right.1));
    Ok(entries)
}

/// Recursively descend into `dir`, appending discovered entries to `out`.
pub(super) fn recurse(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, String)>) -> Result<()> {
    let read_dir = match fs::read_dir(dir) {
        Ok(read_dir) => read_dir,
        Err(error) => return Err(io_error(dir, &error)),
    };

    let mut names: Vec<std::ffi::OsString> = read_dir
        .map(|entry| {
            entry
                .map(|entry| entry.file_name())
                .map_err(|error| io_error(dir, &error))
        })
        .collect::<Result<_>>()?;
    names.sort_unstable();

    for name in names {
        let abs = dir.join(&name);
        let rel = normalize_rel(&abs, root);
        let meta = match fs::symlink_metadata(&abs) {
            Ok(meta) => meta,
            Err(error) => return Err(io_error(&abs, &error)),
        };
        out.push((abs.clone(), rel));
        if meta.is_dir() {
            recurse(root, &abs, out)?;
        }
    }
    Ok(())
}

pub(super) fn symlink_metadata_with_context(abs: &Path) -> Result<std::fs::Metadata> {
    std::fs::symlink_metadata(abs).map_err(|error| {
        ErofsError::Io(std::io::Error::new(
            error.kind(),
            format!("{}: {}", abs.display(), error),
        ))
    })
}

pub(super) fn io_error(path: &Path, error: &std::io::Error) -> ErofsError {
    ErofsError::Walk(format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{entries, normalize_rel, recurse};
    use crate::error::ErofsError;

    #[test]
    fn normalize_rel_path_not_under_prefix() {
        // ARRANGE
        let path = Path::new("/other/path");
        let prefix = Path::new("/source");
        let result = normalize_rel(path, prefix);
        // ACT
        // ASSERT
        assert_eq!(result, "/");
    }

    #[test]
    fn collect_entries_reads_symlink_target() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("tempdir");
        std::os::unix::fs::symlink("/target", dir.path().join("link")).expect("symlink");
        let entries = entries(dir.path()).expect("entries");
        // ACT
        // ASSERT
        assert!(
            entries
                .iter()
                .any(|entry| entry.0.is_symlink() && entry.1 == "/link")
        );
    }

    #[test]
    fn walk_recursive_reports_missing_directory() {
        // ARRANGE
        let root = Path::new("/definitely/missing/erofs-walk-root");
        let mut out = Vec::new();
        let result = recurse(root, root, &mut out);
        // ACT
        // ASSERT
        assert!(matches!(
            result,
            Err(ErofsError::Walk(message)) if message.contains(root.to_string_lossy().as_ref())
        ));
    }
}
