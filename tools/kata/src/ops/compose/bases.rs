//! Carried and output base documents for a composition.

use std::path::Path;

use super::merge::Bases;
use crate::error::{KataError, Result};
use crate::ops::compose::lines;
use crate::repository;
use crate::schema::documents::Document;
use crate::schema::kinds::Kind;

/// Determine the lineage parent line of `release` in `root`.
///
/// # Errors
///
/// Returns an error when the explicit parent is the release being composed,
/// does not exist, or is not the newest eligible line.
pub(crate) fn parent_line(
    root: &Path,
    release: &str,
    explicit: Option<&str>,
) -> Result<Option<String>> {
    let mut eligible = lines::scan(root);
    eligible.retain(|line| line != release);
    let newest = eligible.last().cloned();

    let Some(explicit) = explicit else {
        return Ok(newest);
    };

    if explicit == release {
        return Err(KataError::Lineage(format!(
            "--from-line {explicit} is the line being composed"
        )));
    }
    let Some(newest) = newest else {
        return Err(KataError::Lineage(format!(
            "--from-line {explicit} does not exist"
        )));
    };
    if lines::compare(explicit, &newest) != std::cmp::Ordering::Equal {
        return Err(KataError::Lineage(format!(
            "--from-line {explicit} is not the newest line ({newest})"
        )));
    }

    Ok(Some(explicit.to_owned()))
}

/// Load the scratch documents of `release` in `root`, per kind, when present.
///
/// # Errors
///
/// Returns an error when a document exists but cannot be parsed.
pub(crate) fn output_documents(dir: &Path, release: &str) -> Result<Bases> {
    let core = match optional(Kind::Core, dir, release)? {
        Some(Document::Core(core)) => Some(core),
        Some(_) => return Err(unexpected_kind(Kind::Core, release)),
        None => None,
    };
    let overlays = match optional(Kind::Overlays, dir, release)? {
        Some(Document::Overlays(overlays)) => Some(overlays),
        Some(_) => return Err(unexpected_kind(Kind::Overlays, release)),
        None => None,
    };
    let extensions = match optional(Kind::Extensions, dir, release)? {
        Some(Document::Extensions(extensions)) => Some(extensions),
        Some(_) => return Err(unexpected_kind(Kind::Extensions, release)),
        None => None,
    };

    Ok(Bases {
        core,
        overlays,
        extensions,
    })
}

/// Load the lineage parent documents from `root`, requiring a core document.
///
/// # Errors
///
/// Returns an error when the parent has no core document or a document
/// exists under the wrong kind.
pub(crate) fn parent_bases(root: &Path, line: &str) -> Result<Bases> {
    let core = match optional(Kind::Core, root, line)? {
        Some(Document::Core(core)) => Some(core),
        Some(_) => return Err(unexpected_kind(Kind::Core, line)),
        None => {
            return Err(KataError::Lineage(format!("no core document for {line}")));
        }
    };
    let overlays = match optional(Kind::Overlays, root, line)? {
        Some(Document::Overlays(overlays)) => Some(overlays),
        Some(_) => return Err(unexpected_kind(Kind::Overlays, line)),
        None => None,
    };
    let extensions = match optional(Kind::Extensions, root, line)? {
        Some(Document::Extensions(extensions)) => Some(extensions),
        Some(_) => return Err(unexpected_kind(Kind::Extensions, line)),
        None => None,
    };

    Ok(Bases {
        core,
        overlays,
        extensions,
    })
}

fn optional(kind: Kind, root: &Path, release: &str) -> Result<Option<Document>> {
    if !repository::document_path(kind, root, release).exists() {
        return Ok(None);
    }

    Ok(Some(repository::load(kind, root, release)?))
}

fn unexpected_kind(kind: Kind, line: &str) -> KataError {
    KataError::Document(format!(
        "{}/{line}.toml is not a {} document",
        kind.dir(),
        kind.dir()
    ))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{output_documents, parent_bases, parent_line};
    use crate::repository;
    use crate::schema::documents::{CoreDocument, Document};
    use crate::schema::entries::SourcedEntry;
    use crate::schema::kinds::CORE_API_VERSION;

    fn write_line(root: &TempDir, line: &str) {
        let core = CoreDocument {
            api_version: CORE_API_VERSION.to_owned(),
            release: line.to_owned(),
            kernels: vec![SourcedEntry {
                source: "muak-os/linux".to_owned(),
                repository: "linux".to_owned(),
                tag: "v6".to_owned(),
                digest: "sha256:kernel".to_owned(),
            }],
            stub: None,
            installer: None,
        };
        repository::write(&Document::Core(core), root.path(), line).expect("write core");
    }

    #[test]
    fn parent_defaults_to_the_newest_line_other_than_the_release() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        for line in ["v1.0.0-beta", "v1.1.0"] {
            write_line(&root, line);
        }

        // ACT / ASSERT
        assert_eq!(
            parent_line(root.path(), "v1.1.0", None).expect("parent"),
            Some("v1.0.0-beta".to_owned())
        );
        assert_eq!(
            parent_line(root.path(), "v1.2.0", None).expect("parent"),
            Some("v1.1.0".to_owned())
        );
    }

    #[test]
    fn parent_is_none_without_eligible_lines() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");

        // ACT
        let parent = parent_line(root.path(), "v1.0.0", None).expect("parent");

        // ASSERT
        assert_eq!(parent, None);
    }

    #[test]
    fn explicit_parent_must_be_the_newest_eligible_line() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        for line in ["v1.0.0-beta", "v1.1.0"] {
            write_line(&root, line);
        }

        // ACT / ASSERT
        assert_eq!(
            parent_line(root.path(), "v1.2.0", Some("v1.1.0")).expect("explicit parent"),
            Some("v1.1.0".to_owned())
        );
        let error = parent_line(root.path(), "v1.2.0", Some("v1.0.0-beta"))
            .expect_err("stale parent must fail");
        assert!(
            error
                .to_string()
                .contains("is not the newest line (v1.1.0)")
        );
    }

    #[test]
    fn explicit_parent_refuses_the_release_and_stale_lines() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        write_line(&root, "v1.0.0");

        // ACT / ASSERT
        let error =
            parent_line(root.path(), "v1.1.0", Some("v1.1.0")).expect_err("self parent must fail");
        assert!(error.to_string().contains("is the line being composed"));
        let error = parent_line(root.path(), "v1.1.0", Some("v9.9.9"))
            .expect_err("unknown parent must fail");
        assert!(
            error
                .to_string()
                .contains("is not the newest line (v1.0.0)")
        );
    }

    #[test]
    fn explicit_parent_requires_an_existing_line() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");

        // ACT / ASSERT
        let error = parent_line(root.path(), "v1.1.0", Some("v9.9.9"))
            .expect_err("missing parent must fail");
        assert!(error.to_string().contains("does not exist"));
    }

    #[test]
    fn parent_bases_require_a_core_document() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");

        // ACT / ASSERT
        let error = parent_bases(root.path(), "v1.0.0").expect_err("missing parent core");
        assert!(error.to_string().contains("no core document for v1.0.0"));
    }

    #[test]
    fn output_documents_load_same_line_documents() {
        // ARRANGE
        let root = TempDir::new().expect("create temp dir");
        write_line(&root, "v1.1.0");

        // ACT
        let bases = output_documents(root.path(), "v1.1.0").expect("output documents");

        // ASSERT
        assert!(bases.core.is_some());
        assert!(bases.overlays.is_none());
        assert!(bases.extensions.is_none());
    }
}
