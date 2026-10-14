//! Entry parsing and validation for pushed files.

use std::path::{Path, PathBuf};

use super::Entry;
use crate::error::{KociError, Result};

impl Entry {
    /// Parse a `PATH[:ARCHIVE_PATH]` entry specification such as `catalog.toml`.
    ///
    /// # Errors
    ///
    /// Returns an error when the source path or the archive path ends up empty.
    pub fn parse(spec: &str) -> Result<Self> {
        let (source, archive_path) = match spec.split_once(':') {
            Some((source, archive_path)) => (source, archive_path),
            None => (spec, ""),
        };
        if source.is_empty() {
            return Err(spec_error(spec, "source path is empty"));
        }

        let path = match archive_path {
            "" => file_name_of(source, spec)?,
            path => path.to_owned(),
        };

        Ok(Self {
            path,
            source: PathBuf::from(source),
        })
    }

    /// Reject invalid and duplicate archive paths before any registry call.
    ///
    /// # Errors
    ///
    /// Returns an error for empty, absolute, parent-traversing, or duplicate paths.
    pub(super) fn validate(entries: &[Self]) -> Result<()> {
        for entry in entries {
            validate_path(&entry.path)?;
        }
        if let Some(path) = duplicate(entries) {
            return Err(KociError::PushError(format!(
                "duplicate archive path '{path}'"
            )));
        }

        Ok(())
    }
}

fn duplicate(entries: &[Entry]) -> Option<&str> {
    entries.iter().enumerate().find_map(|(position, entry)| {
        entries
            .iter()
            .take(position)
            .any(|other| other.path == entry.path)
            .then_some(entry.path.as_str())
    })
}

fn file_name_of(source: &str, spec: &str) -> Result<String> {
    Path::new(source)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .ok_or_else(|| spec_error(spec, "cannot derive archive path from source"))
}

fn validate_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.starts_with('/')
        || path.split('/').any(|component| component == "..")
    {
        return Err(KociError::PushError(format!(
            "archive path must be relative and traversal-free, got '{path}'"
        )));
    }

    Ok(())
}

fn spec_error(spec: &str, details: &str) -> KociError {
    KociError::PushError(format!("invalid file spec '{spec}': {details}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_defaults_archive_path_to_file_name() {
        // ARRANGE
        let spec = "/etc/muak/catalog.toml";

        // ACT
        let entry = Entry::parse(spec).expect("parse spec");

        // ASSERT
        assert_eq!(entry.path, "catalog.toml");
        assert_eq!(entry.source, PathBuf::from("/etc/muak/catalog.toml"));
    }

    #[test]
    fn parse_honors_explicit_archive_path_without_suffix() {
        // ARRANGE
        let spec = "out/catalog.toml";

        // ACT
        let entry = Entry::parse(spec).expect("parse spec");

        // ASSERT
        assert_eq!(entry.path, "catalog.toml");
    }

    #[test]
    fn parse_rejects_empty_and_unnameable_sources() {
        // ARRANGE / ACT / ASSERT
        let error = parse_error(":name.toml");
        assert!(matches!(error, KociError::PushError(_)));

        let error = parse_error("/");
        assert!(matches!(error, KociError::PushError(_)));
    }

    #[test]
    fn validate_rejects_dot_components_anywhere_in_the_path() {
        // ARRANGE
        let entries = [Entry {
            path: "ok/../bad.toml".to_owned(),
            source: PathBuf::from("x"),
        }];

        // ACT / ASSERT
        assert!(Entry::validate(&entries).is_err());
    }

    fn parse_error(spec: &str) -> KociError {
        Entry::parse(spec).expect_err("spec should be rejected")
    }
}
