//! Carried and output base documents for a composition.

use std::path::{Path, PathBuf};

use super::merge::Bases;
use crate::error::{KataError, Result};
use crate::ops::compose::lines;
use crate::repository;
use crate::schema::documents::Document;
use crate::schema::kinds::Kind;

/// The derivation root and line the composition carries pins from.
pub(crate) struct Carried {
    pub(crate) root: PathBuf,
    pub(crate) line: String,
}

/// Load the lineage parent documents from the derivation root, if any.
///
/// # Errors
///
/// Returns an error when the lineage rule is violated or the derivation root
/// carries no core document for the parent line.
pub(crate) fn carried_documents(
    from: Option<&PathBuf>,
    from_line: Option<&str>,
) -> Result<Option<Carried>> {
    let Some(root) = from else {
        return Ok(None);
    };

    let lines = lines::scan(root);
    let line = match from_line {
        Some(explicit) => {
            check_lineage(&lines, explicit)?;
            explicit.to_owned()
        }
        None => lines::newest(root).ok_or_else(|| {
            KataError::Lineage(format!(
                "the derivation root {} carries no lines",
                root.display()
            ))
        })?,
    };
    if !repository::document_path(Kind::Core, root, &line).exists() {
        return Err(KataError::Lineage(format!(
            "the derivation root {} carries no core document for {line}",
            root.display()
        )));
    }

    Ok(Some(Carried {
        root: root.clone(),
        line,
    }))
}

/// Load the scratch documents of the output root, per kind, when present.
#[must_use]
pub(crate) fn output_documents(dir: &Path, release: &str) -> Bases {
    let core = optional(Kind::Core, dir, release).and_then(|document| match document {
        Document::Core(core) => Some(core),
        Document::Overlays(_) | Document::Extensions(_) => None,
    });
    let overlays = optional(Kind::Overlays, dir, release).and_then(|document| match document {
        Document::Overlays(overlays) => Some(overlays),
        Document::Core(_) | Document::Extensions(_) => None,
    });
    let extensions = optional(Kind::Extensions, dir, release).and_then(|document| match document {
        Document::Extensions(extensions) => Some(extensions),
        Document::Core(_) | Document::Overlays(_) => None,
    });

    Bases {
        core,
        overlays,
        extensions,
    }
}

/// Load the carried parent documents, requiring a core document.
///
/// # Errors
///
/// Returns an error when a document exists under the wrong kind or the core document is missing.
pub(crate) fn carried_bases(carried: &Carried) -> Result<Bases> {
    let root = &carried.root;
    let line = &carried.line;

    let core = match optional(Kind::Core, root, line) {
        Some(Document::Core(core)) => Some(core),
        Some(_) => {
            return Err(KataError::Document(format!(
                "core/{line}.toml is not a core document"
            )));
        }
        None => {
            return Err(KataError::Document(format!(
                "the derivation root carries no core document for {line}"
            )));
        }
    };
    let overlays = match optional(Kind::Overlays, root, line) {
        Some(Document::Overlays(overlays)) => Some(overlays),
        Some(_) => {
            return Err(KataError::Document(format!(
                "overlays/{line}.toml is not an overlays document"
            )));
        }
        None => None,
    };
    let extensions = match optional(Kind::Extensions, root, line) {
        Some(Document::Extensions(extensions)) => Some(extensions),
        Some(_) => {
            return Err(KataError::Document(format!(
                "extensions/{line}.toml is not an extensions document"
            )));
        }
        None => None,
    };

    Ok(Bases {
        core,
        overlays,
        extensions,
    })
}

fn check_lineage(lines: &[String], explicit: &str) -> Result<()> {
    let Some(newest) = lines.last() else {
        return Ok(());
    };
    if lines::compare(explicit, newest) != std::cmp::Ordering::Less {
        return Ok(());
    }

    Err(KataError::Lineage(format!(
        "--from-line {explicit} is not the newest line ({newest})"
    )))
}

fn optional(kind: Kind, root: &Path, release: &str) -> Option<Document> {
    if repository::document_path(kind, root, release).exists() {
        repository::load(kind, root, release).ok()
    } else {
        None
    }
}
