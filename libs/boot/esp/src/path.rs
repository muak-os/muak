//! Path validation helpers for ESP-relative file entries.

use std::path::{Component, Path};

use crate::error::{EspError, Result};

/// Validates an iterator of ESP-relative file paths.
///
/// # Errors
///
/// Returns `EspError::InvalidPath` if any path is invalid.
pub fn validate_spec<'a, I: Iterator<Item = &'a str>>(paths: I) -> Result<()> {
    for path in paths {
        validate_relative(path)?;
    }

    Ok(())
}

/// Validates an ESP-relative path and returns it as a `Path`.
///
/// # Errors
///
/// Returns `EspError::InvalidPath` if the path is empty, absolute, contains
/// unsupported components, or does not contain a file name.
pub fn validate_relative(path: &str) -> Result<&Path> {
    let rel_path = Path::new(path);
    if path.is_empty() {
        return Err(EspError::InvalidPath("path is empty".to_owned()));
    }
    if rel_path.is_absolute() {
        return Err(EspError::InvalidPath(format!(
            "path must be relative: {path}"
        )));
    }

    let mut has_normal_component = false;
    for component in rel_path.components() {
        match component {
            Component::Normal(_) => {
                has_normal_component = true;
            }
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir | Component::ParentDir => {
                return Err(EspError::InvalidPath(format!(
                    "path contains unsupported component: {path}"
                )));
            }
        }
    }

    if !has_normal_component {
        return Err(EspError::InvalidPath(format!(
            "path does not contain a file name: {path}"
        )));
    }

    Ok(rel_path)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::validate_relative;
    use crate::error::EspError;

    #[test]
    fn validate_relative_path_accepts_nested_relative_paths() {
        // ARRANGE / ACT
        let result = validate_relative("EFI/BOOT/BOOTX64.EFI");

        // ASSERT
        assert_eq!(
            result.expect("path must validate"),
            Path::new("EFI/BOOT/BOOTX64.EFI")
        );
    }

    #[test]
    fn validate_relative_path_rejects_empty_paths() {
        // ARRANGE / ACT
        let result = validate_relative("");

        // ASSERT
        assert!(matches!(result, Err(EspError::InvalidPath(_))));
    }

    #[test]
    fn validate_relative_path_rejects_absolute_paths() {
        // ARRANGE / ACT
        let result = validate_relative("/EFI/BOOT/BOOTX64.EFI");

        // ASSERT
        assert!(matches!(result, Err(EspError::InvalidPath(_))));
    }

    #[test]
    fn validate_relative_path_rejects_parent_traversal() {
        // ARRANGE / ACT
        let result = validate_relative("../escape");

        // ASSERT
        assert!(matches!(result, Err(EspError::InvalidPath(_))));
    }

    #[test]
    fn validate_relative_path_rejects_directory_only_path() {
        // ARRANGE / ACT
        let result = validate_relative(".");

        // ASSERT
        assert!(matches!(result, Err(EspError::InvalidPath(_))));
    }
}
