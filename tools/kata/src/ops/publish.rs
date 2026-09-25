//! Building and pushing catalog OCI images.

use alloc::collections::BTreeMap;
use std::path::Path;

use koci::arch::Arch;
use koci::registry;
use koci::{pull, push};

use crate::error::{KataError, Result};
use crate::ops::verify;
use crate::repository;
use crate::schema::documents::Document;
use crate::schema::kinds::Kind;
use crate::schema::parse::{from_toml, validate_release};

const DOCUMENT_PATH: &str = "catalog.toml";

/// Publish one kind's catalog image, or every kind with a document for the
/// release when `kind` is [`None`].
///
/// # Errors
///
/// Returns an error when the root is unusable, no document exists, a document
/// is not canonical, an entry fails verification, a published line would be
/// mutated, or the registry push fails.
pub fn run(
    root: &Path,
    release: &str,
    kind: Option<Kind>,
    registry: &str,
    arch: Arch,
    force: bool,
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
        published.push(publish_one(root, release, kind, registry, arch, force)?);
    }

    Ok(published)
}

fn served_document(line_reference: &str, kind: Kind, release: &str) -> Result<Option<Document>> {
    let mut bytes: Option<Vec<u8>> = None;
    pull::files(line_reference, &Arch::Amd64, None, |entry| {
        if entry.path == DOCUMENT_PATH {
            let mut buffer = Vec::new();
            entry.reader.read_to_end(&mut buffer)?;
            bytes = Some(buffer);
        }

        Ok(())
    })
    .map_err(|error| KataError::Registry(error.to_string()))?;

    match bytes {
        Some(bytes) => Ok(Some(from_toml(kind, &bytes, release)?)),
        None => Ok(None),
    }
}

fn check_monotonic(kind: Kind, document: &Document, served: &Document) -> Result<()> {
    let mut served_entries = BTreeMap::new();
    for check in verify::checks(served) {
        served_entries.insert(format!("{}/{}", kind.dir(), check.identity), check);
    }

    for check in verify::checks(document) {
        let identity = format!("{}/{}", kind.dir(), check.identity);
        match served_entries.remove(&identity) {
            Some(existing) if existing != check => {
                return Err(KataError::Gate(format!(
                    "published entry '{identity}' changed ({} {} → {} {}); \
                     published pins never change — compose a new line",
                    existing.tag, existing.digest, check.tag, check.digest
                )));
            }
            _ => {}
        }
    }

    if !served_entries.is_empty() {
        let removed: Vec<String> = served_entries.into_keys().collect();
        return Err(KataError::Gate(format!(
            "published entries would be removed: {} — removals land in the next line",
            removed.join(", ")
        )));
    }

    Ok(())
}

fn publish_one(
    root: &Path,
    release: &str,
    kind: Kind,
    registry: &str,
    arch: Arch,
    force: bool,
) -> Result<String> {
    let document = repository::load(kind, root, release)?;
    if !repository::canonical_on_disk(&document, root, release)? {
        return Err(KataError::Document(format!(
            "{}/{} is not in canonical form; regenerate it before publishing",
            kind.dir(),
            release
        )));
    }

    verify::verify_document(&document, registry)?;

    let line_reference = format!("{registry}/{}:{release}", kind.repository());
    let line_published = registry::manifest_exists(&line_reference)
        .map_err(|error| KataError::Registry(error.to_string()))?;

    if !force
        && line_published
        && let Some(served) = served_document(&line_reference, kind, release)?
    {
        check_monotonic(kind, &document, &served)?;
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
