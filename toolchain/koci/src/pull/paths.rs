//! Tar entry path normalization and whiteout naming.

use std::path::{Component, Path, PathBuf};

use crate::error::{KociError, Result};

/// Normalize a tar entry path, rejecting parent traversal and skipping `.` / root entries.
pub(crate) fn normalize_entry_path(path: &Path) -> Result<Option<PathBuf>> {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir | Component::RootDir => {}
            Component::ParentDir => {
                return Err(KociError::LayerExtractionError(format!(
                    "OCI layer entry escapes extraction root: {}",
                    path.display()
                )));
            }
            Component::Prefix(prefix) => {
                #[cfg(windows)]
                {
                    let _ = prefix;
                    return Err(KociError::LayerExtractionError(format!(
                        "OCI layer entry uses unsupported path prefix: {}",
                        path.display()
                    )));
                }

                #[cfg(not(windows))]
                normalized.push(prefix.as_os_str());
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalized))
    }
}

/// If `path` is a whiteout entry, return the target path that should be removed.
pub(crate) fn whiteout_target(path: &Path) -> Option<PathBuf> {
    let file_name = path.file_name().and_then(|name| name.to_str())?;

    if file_name == ".wh..wh..opq" {
        return Some(path.parent().unwrap_or_else(|| Path::new("")).to_path_buf());
    }

    let stripped = file_name.strip_prefix(".wh.")?;
    let parent = path.parent().unwrap_or_else(|| Path::new(""));

    Some(parent.join(stripped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_entry_path_returns_none_for_current_directory() {
        // ARRANGE
        let path = Path::new("./");

        // ACT
        let normalized = normalize_entry_path(path).expect("normalize path");

        // ASSERT
        assert!(normalized.is_none());
    }

    #[test]
    fn normalize_entry_path_rejects_parent_traversal() {
        // ACT
        let error =
            normalize_entry_path(Path::new("../escape")).expect_err("normalize should fail");

        // ASSERT
        assert!(matches!(error, KociError::LayerExtractionError(_)));
    }

    #[test]
    fn whiteout_target_returns_none_for_non_whiteout_path() {
        // ACT
        let target = whiteout_target(Path::new("etc/file"));

        // ASSERT
        assert!(target.is_none());
    }

    #[test]
    fn whiteout_target_returns_file_target() {
        // ACT
        let target = whiteout_target(Path::new("etc/.wh.obsolete"));

        // ASSERT
        assert_eq!(target, Some(PathBuf::from("etc/obsolete")));
    }

    #[test]
    fn whiteout_target_returns_opaque_directory_target() {
        // ACT
        let target = whiteout_target(Path::new("etc/.wh..wh..opq"));

        // ASSERT
        assert_eq!(target, Some(PathBuf::from("etc")));
    }
}
