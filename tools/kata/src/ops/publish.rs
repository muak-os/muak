//! Building and pushing catalog OCI images.

use std::path::Path;

use koci::arch::Arch;
use koci::push;

use crate::error::{KataError, Result};
use crate::repository;
use crate::schema::kinds::Kind;
use crate::schema::parse::validate_release;

/// Publish one kind's catalog image, or every kind with a document for the
/// release when `kind` is [`None`].
///
/// # Errors
///
/// Returns an error when the root is unusable, no document exists, a document
/// is not canonical, or the registry push fails.
pub fn run(
    root: &Path,
    release: &str,
    kind: Option<Kind>,
    registry: &str,
    arch: Arch,
) -> Result<Vec<String>> {
    repository::require_root(root)?;
    validate_release(release)?;
    let kinds = match kind {
        Some(kind) => vec![kind],
        None => Kind::all()
            .into_iter()
            .filter(|kind| repository::document_path(*kind, root, release).exists())
            .collect(),
    };
    if kinds.is_empty() {
        return Err(KataError::Document(format!(
            "no catalog documents found for {release}"
        )));
    }

    let mut published = Vec::new();
    for kind in kinds {
        published.push(publish_one(root, release, kind, registry, arch)?);
    }

    Ok(published)
}

fn publish_one(
    root: &Path,
    release: &str,
    kind: Kind,
    registry: &str,
    arch: Arch,
) -> Result<String> {
    let document = repository::load(kind, root, release)?;
    if !repository::canonical_on_disk(&document, root, release)? {
        return Err(KataError::Document(format!(
            "{}/{} is not in canonical form; regenerate it before publishing",
            kind.dir(),
            release
        )));
    }

    let image = format!("{registry}/{}", kind.repository());
    let pushed = push::files(
        &image,
        &[release.to_owned()],
        &arch,
        &[push::Entry {
            path: "catalog.toml".to_owned(),
            source: repository::document_path(kind, root, release),
        }],
    )
    .map_err(|error| KataError::Registry(error.to_string()))?;
    eprintln!("Published {image}:{release}");

    Ok(pushed.digest)
}
