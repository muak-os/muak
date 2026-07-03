//! Path validation helpers for ESP-relative file entries.

use std::path::{Component, Path};

use crate::error::{EspError, Result};
use crate::model::EspSpec;

/// Validates all paths in an `EspSpec`.
pub(crate) fn validate_spec(spec: &EspSpec<'_>) -> Result<()> {
    for file in spec.files() {
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

/// Validates an iterator of ESP-relative file paths.
pub(crate) fn validate_spec_paths<'a>(paths: impl Iterator<Item = &'a str>) -> Result<()> {
    for path in paths {
        validate_relative_path(path)?;
    }

    Ok(())
}

/// Validates an ESP-relative path and returns its normalized string form.
pub(crate) fn normalize_relative_path(path: &str) -> Result<String> {
    let rel_path = validate_relative_path(path)?;
    let mut components = Vec::new();
    for component in rel_path.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        components.push(name.to_string_lossy().into_owned());
    }

    let normalized = components.join("/");

    Ok(normalized)
}

/// Validates an ESP-relative path and returns it as a `Path`.
pub(crate) fn validate_relative_path(path: &str) -> Result<&Path> {
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
    use std::io::Cursor;
    use std::path::Path;

    use super::{normalize_relative_path, validate_relative_path, validate_spec};
    use crate::error::EspError;
    use crate::model::{EspFile, EspSpec};

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
    fn validate_spec_accepts_normalized_paths() {
        // ARRANGE
        let mut cursor = Cursor::new(Vec::<u8>::new());
        let spec = EspSpec::builder()
            .add_file(EspFile {
                path: "valid/file".to_owned(),
                reader: &mut cursor,
                size: 0,
            })
            .expect("file must be added")
            .build()
            .expect("spec must build");

        // ACT
        let result = validate_spec(&spec);

        // ASSERT
        result.expect("normalized spec must validate");
    }

    #[test]
    fn validate_spec_rejects_non_normalized_paths() {
        // ARRANGE
        let mut cursor = Cursor::new(Vec::<u8>::new());
        let mut spec = EspSpec::builder()
            .add_file(EspFile {
                path: "EFI/BOOT/BOOTX64.EFI".to_owned(),
                reader: &mut cursor,
                size: 0,
            })
            .expect("file must be added")
            .build()
            .expect("spec must build");
        spec.files_mut().first_mut().expect("file must exist").path =
            "./EFI/BOOT/BOOTX64.EFI".to_owned();

        // ACT
        let result = validate_spec(&spec);

        // ASSERT
        assert!(matches!(result, Err(EspError::InvalidPath(_))));
    }
}
