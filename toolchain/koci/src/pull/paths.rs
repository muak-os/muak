//! Tar entry path normalization and whiteout naming.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use crate::error::{KociError, Result};

/// File name suffix marking a whiteout entry.
const WHITEOUT_PREFIX: &str = ".wh.";

/// File name marking an opaque directory whiteout.
const OPAQUE_MARKER: &str = ".wh..wh..opq";

/// Normalize a tar entry path into `buf`, rejecting parent traversal.
///
/// # Errors
///
/// Returns an error when the path escapes the extraction root.
pub(crate) fn normalize_entry_path_into(path: &Path, buf: &mut PathBuf) -> Result<bool> {
    buf.clear();

    for component in path.components() {
        match component {
            Component::Normal(part) => buf.push(part),
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
                buf.push(prefix.as_os_str());
            }
        }
    }

    Ok(!buf.as_os_str().is_empty())
}

/// If `buf` holds a whiteout entry, rewrite it into the target path that should be hidden.
pub(crate) fn whiteout_target_into(buf: &mut PathBuf) -> bool {
    let Some(file_name) = buf.file_name().and_then(OsStr::to_str) else {
        return false;
    };

    let (parent, stripped) = if file_name == OPAQUE_MARKER {
        (buf.parent(), None)
    } else {
        match file_name.strip_prefix(WHITEOUT_PREFIX) {
            Some(stripped) => (buf.parent(), Some(stripped)),
            None => return false,
        }
    };

    let parent = parent.unwrap_or_else(|| Path::new("")).to_path_buf();
    let stripped = stripped.map(str::to_owned);

    buf.clear();
    if !parent.as_os_str().is_empty() {
        buf.push(parent);
    }
    if let Some(stripped) = stripped {
        buf.push(stripped);
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normalized(path: &str) -> Option<PathBuf> {
        let mut buf = PathBuf::new();
        normalize_entry_path_into(Path::new(path), &mut buf)
            .expect("normalize path")
            .then_some(buf)
    }

    #[test]
    fn normalize_entry_path_into_keeps_the_buffer_reusable() {
        // ARRANGE
        let mut buf = PathBuf::from("previous/entry");

        // ACT
        let first =
            normalize_entry_path_into(Path::new("./etc/motd"), &mut buf).expect("normalize");
        let second = normalize_entry_path_into(Path::new("./"), &mut buf).expect("normalize");

        // ASSERT
        assert!(first, "the first entry must normalize");
        assert!(!second, "the root entry must be skipped");
    }

    #[test]
    fn normalize_entry_path_returns_none_for_current_directory() {
        // ACT / ASSERT
        assert!(normalized("./").is_none());
    }

    #[test]
    fn normalize_entry_path_rejects_parent_traversal() {
        // ACT
        let mut buf = PathBuf::new();
        let error =
            normalize_entry_path_into(Path::new("../escape"), &mut buf).expect_err("normalize");

        // ASSERT
        assert!(matches!(error, KociError::LayerExtractionError(_)));
    }

    #[test]
    fn whiteout_target_into_returns_false_for_non_whiteout_path() {
        // ARRANGE
        let mut buf = PathBuf::from("etc/file");

        // ACT
        let is_whiteout = whiteout_target_into(&mut buf);

        // ASSERT
        assert!(!is_whiteout);
        assert_eq!(buf, PathBuf::from("etc/file"));
    }

    #[test]
    fn whiteout_target_into_rewrites_the_file_target() {
        // ARRANGE
        let mut buf = PathBuf::from("etc/.wh.obsolete");

        // ACT
        let is_whiteout = whiteout_target_into(&mut buf);

        // ASSERT
        assert!(is_whiteout);
        assert_eq!(buf, PathBuf::from("etc/obsolete"));
    }

    #[test]
    fn whiteout_target_into_rewrites_the_opaque_directory_target() {
        // ARRANGE
        let mut buf = PathBuf::from("etc/.wh..wh..opq");

        // ACT
        let is_whiteout = whiteout_target_into(&mut buf);

        // ASSERT
        assert!(is_whiteout);
        assert_eq!(buf, PathBuf::from("etc"));
    }

    #[test]
    fn whiteout_target_into_handles_root_level_entries() {
        // ARRANGE
        let mut file_buf = PathBuf::from(".wh.vmlinuz");
        let mut opaque_buf = PathBuf::from(".wh..wh..opq");

        // ACT
        let file_whiteout = whiteout_target_into(&mut file_buf);
        let opaque_whiteout = whiteout_target_into(&mut opaque_buf);

        // ASSERT
        assert!(file_whiteout);
        assert_eq!(file_buf, PathBuf::from("vmlinuz"));
        assert!(opaque_whiteout);
        assert!(opaque_buf.as_os_str().is_empty());
    }
}
