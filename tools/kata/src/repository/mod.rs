//! Publisher-side repository layout.

use std::path::{Path, PathBuf};

use crate::error::{KataError, Result};
use crate::schema::documents::Document;
use crate::schema::kinds::Kind;
use crate::schema::parse::from_toml;
use crate::schema::serialize::canonical;

pub(crate) mod edit;

/// Ensure `root` names a usable catalog repository layout.
///
/// # Errors
///
/// Returns an error when `root` is not a directory.
pub fn require_root(root: &Path) -> Result<()> {
    if root.is_dir() {
        return Ok(());
    }

    Err(KataError::Document(format!(
        "catalog repository root '{}' is not a directory",
        root.display()
    )))
}

/// Path of one kind's document for `release` under the repository root.
#[must_use]
pub fn document_path(kind: Kind, root: &Path, release: &str) -> PathBuf {
    root.join(kind.dir()).join(format!("{release}.toml"))
}

/// Load the document of `kind` for `release` under the repository `root`.
///
/// # Errors
///
/// Returns an error when the file is missing, fails to parse, or carries an
/// unexpected `api_version` or `release`.
pub fn load(kind: Kind, root: &Path, release: &str) -> Result<Document> {
    let bytes = std::fs::read(document_path(kind, root, release))?;

    Ok(from_toml(kind, &bytes, release)?)
}

/// Write the document in canonical form; returns the written path.
///
/// # Errors
///
/// Returns an error when serialization or the filesystem write fails.
pub fn write(document: &Document, root: &Path, release: &str) -> Result<PathBuf> {
    let path = document_path(document.kind(), root, release);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, canonical(document)?)?;

    Ok(path)
}

/// Whether the file on disk already is the canonical form.
///
/// # Errors
///
/// Returns an error when the file cannot be read or serialization fails.
pub fn canonical_on_disk(document: &Document, root: &Path, release: &str) -> Result<bool> {
    let on_disk = std::fs::read_to_string(document_path(document.kind(), root, release))?;

    Ok(on_disk == canonical(document)?)
}
