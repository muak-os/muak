//! HEAD-verifying pinned entries against the registry.

use std::path::Path;

use koci::registry;

use crate::error::{KataError, Result};
use crate::repository;
use crate::schema::documents::Document;
use crate::schema::entries::NamedEntry;
use crate::schema::kinds::Kind;
use crate::schema::parse::validate_release;

/// One document entry to recheck against the registry.
struct EntryCheck {
    /// Human identity in messages (`source` or `name`).
    identity: String,
    /// Repository path relative to the registry prefix.
    repository: String,
    /// Tag the digest was resolved from.
    tag: String,
    /// Digest the document pins.
    digest: String,
}

/// Verify one release line, or every existing line when `release` is [`None`].
///
/// # Errors
///
/// Returns an error listing every failing line.
pub fn run(root: &Path, release: Option<&str>, registry: &str) -> Result<Vec<String>> {
    repository::require_root(root)?;
    if let Some(release) = release {
        validate_release(release)?;
    }
    let mut verified = Vec::new();
    let mut failures = Vec::new();
    for (kind, line) in lines(root, release) {
        match verify_line(root, kind, &line, registry) {
            Ok(count) => verified.push(format!("{}/{} ({count} entries)", kind.dir(), line)),
            Err(error) => failures.push(format!("{}/{line}: {error}", kind.dir())),
        }
    }

    if failures.is_empty() {
        Ok(verified)
    } else {
        Err(KataError::Document(format!(
            "verification failures:\n{}",
            failures.join("\n")
        )))
    }
}

fn lines(root: &Path, release: Option<&str>) -> Vec<(Kind, String)> {
    let mut found = Vec::new();
    for kind in Kind::all() {
        match release {
            Some(line) => collect_line(kind, root, line, &mut found),
            None => collect_all(kind, root, &mut found),
        }
    }

    found.sort_by(|left, right| left.1.cmp(&right.1));

    found
}

fn collect_line(kind: Kind, root: &Path, line: &str, found: &mut Vec<(Kind, String)>) {
    if repository::document_path(kind, root, line).exists() {
        found.push((kind, line.to_owned()));
    }
}

fn collect_all(kind: Kind, root: &Path, found: &mut Vec<(Kind, String)>) {
    let Ok(entries) = std::fs::read_dir(root.join(kind.dir())) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "toml") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        found.push((kind, stem.to_owned()));
    }
}

fn verify_line(root: &Path, kind: Kind, release: &str, registry: &str) -> Result<usize> {
    let document = repository::load(kind, root, release)?;
    if !repository::canonical_on_disk(&document, root, release)? {
        return Err(KataError::Document(
            "document is not in canonical form; regenerate it".to_owned(),
        ));
    }

    let mut verified: Vec<String> = Vec::new();
    for check in checks(&document) {
        let reference = super::reference(registry, &check.repository, &check.tag);
        let actual = registry::manifest_digest(&reference)
            .map_err(|error| KataError::Registry(error.to_string()))?;
        if actual != check.digest {
            return Err(KataError::Document(format!(
                "entry '{}' pins {} but registry serves {actual} for {reference}",
                check.identity, check.digest
            )));
        }
        verified.push(check.identity);
    }
    eprintln!("{}/{release}: verified", kind.dir());

    Ok(verified.len())
}

fn checks(document: &Document) -> Vec<EntryCheck> {
    match *document {
        Document::Core(ref core) => core
            .kernels
            .iter()
            .chain(core.stub.iter())
            .chain(core.installer.iter())
            .map(|entry| EntryCheck {
                identity: entry.source.clone(),
                repository: entry.repository.clone(),
                tag: entry.tag.clone(),
                digest: entry.digest.clone(),
            })
            .collect(),
        Document::Overlays(ref overlays) => flatten_named(&overlays.overlays),
        Document::Extensions(ref extensions) => flatten_named(&extensions.extensions),
    }
}

fn flatten_named(entries: &[NamedEntry]) -> Vec<EntryCheck> {
    entries
        .iter()
        .map(|entry| EntryCheck {
            identity: entry.name.clone(),
            repository: entry.repository.clone(),
            tag: entry.tag.clone(),
            digest: entry.digest.clone(),
        })
        .collect()
}
