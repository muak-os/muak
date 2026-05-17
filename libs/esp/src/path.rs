//! Path validation helpers for ESP-relative file entries.

use std::path::{Component, Path};

use crate::EspError;

/// Validates all paths in an `EspSpec`.
pub(crate) fn validate_spec(spec: &crate::EspSpec) -> Result<(), EspError> {
    for file in &spec.files {
        let normalized = normalize_relative_path(&file.path)?;
        if normalized != file.path {
            return Err(EspError::InvalidPath(format!(
                "path is not normalized: {}",
                file.path
            )));
        }
    }
    Ok(())
}

/// Validates an ESP-relative path and returns its normalized string form.
pub(crate) fn normalize_relative_path(path: &str) -> Result<String, EspError> {
    let rel_path = validate_relative_path(path)?;
    let mut components = Vec::new();
    for component in rel_path.components() {
        if let Component::Normal(name) = component {
            let name = name
                .to_str()
                .ok_or_else(|| EspError::InvalidPath(format!("non-UTF-8 path: {path}")))?;
            components.push(name);
        }
    }

    let normalized = components.join("/");
    if normalized.is_empty() {
        return Err(EspError::InvalidPath(format!(
            "path does not contain a file name: {path}"
        )));
    }

    Ok(normalized)
}

/// Validates an ESP-relative path and returns it as a `Path`.
pub(crate) fn validate_relative_path(path: &str) -> Result<&Path, EspError> {
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
            _ => {
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

    use super::{normalize_relative_path, validate_relative_path, validate_spec};
    use crate::{EspError, EspFile, EspSpec};

    #[test]
    fn validate_relative_path_accepts_nested_relative_paths() {
        // ARRANGE / ACT
        let result = validate_relative_path("EFI/BOOT/BOOTX64.EFI");

        // ASSERT
        assert_eq!(
            result.expect("path must validate"),
            Path::new("EFI/BOOT/BOOTX64.EFI")
        );
    }

    #[test]
    fn normalize_relative_path_strips_curdir_component() {
        // ARRANGE / ACT
        let result = normalize_relative_path("./EFI/BOOT/BOOTX64.EFI");

        // ASSERT
        assert_eq!(result.expect("path must normalize"), "EFI/BOOT/BOOTX64.EFI");
    }

    #[test]
    fn validate_relative_path_rejects_empty_paths() {
        // ARRANGE / ACT
        let result = validate_relative_path("");

        // ASSERT
        assert!(matches!(result, Err(EspError::InvalidPath(_))));
    }

    #[test]
    fn validate_relative_path_rejects_absolute_paths() {
        // ARRANGE / ACT
        let result = validate_relative_path("/EFI/BOOT/BOOTX64.EFI");

        // ASSERT
        assert!(matches!(result, Err(EspError::InvalidPath(_))));
    }

    #[test]
    fn validate_relative_path_rejects_parent_traversal() {
        // ARRANGE / ACT
        let result = validate_relative_path("../escape");

        // ASSERT
        assert!(matches!(result, Err(EspError::InvalidPath(_))));
    }

    #[test]
    fn validate_relative_path_rejects_directory_only_path() {
        // ARRANGE / ACT
        let result = validate_relative_path(".");

        // ASSERT
        assert!(matches!(result, Err(EspError::InvalidPath(_))));
    }

    #[test]
    fn validate_spec_checks_all_entries() {
        // ARRANGE
        let spec = EspSpec {
            files: vec![
                EspFile {
                    path: "valid/file".to_owned(),
                    data: vec![],
                },
                EspFile {
                    path: "../escape".to_owned(),
                    data: vec![],
                },
            ],
        };

        // ACT
        let result = validate_spec(&spec);

        // ASSERT
        assert!(matches!(result, Err(EspError::InvalidPath(_))));
    }

    #[test]
    fn validate_spec_rejects_non_normalized_paths() {
        // ARRANGE
        let spec = EspSpec {
            files: vec![EspFile {
                path: "./EFI/BOOT/BOOTX64.EFI".to_owned(),
                data: vec![],
            }],
        };

        // ACT
        let result = validate_spec(&spec);

        // ASSERT
        assert!(matches!(result, Err(EspError::InvalidPath(_))));
    }
}
