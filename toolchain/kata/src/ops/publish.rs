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
use crate::schema::parse::{from_toml, from_toml_tag, release_version, validate_release};
use crate::version;

const DOCUMENT_PATH: &str = "catalog.toml";

/// Publish every kind's catalog image with a document for `release`, moving
/// each `channels` tag to it.
///
/// # Errors
///
/// Returns an error when the root is unusable, no document exists, a document is not canonical,
/// an entry fails verification, a published line would be mutated, a channel would move backwards,
/// or the registry push fails.
pub fn run(root: &Path, release: &str, channels: &[String], force: bool) -> Result<Vec<String>> {
    repository::require_root(root)?;
    validate_release(release)?;
    for channel in channels {
        validate_release(channel)?;
    }
    if !force {
        let core = super::reference(Kind::Core.dir(), release);
        let line_published = registry::manifest_exists(&core)
            .map_err(|error| KataError::Registry(error.to_string()))?;
        version::ensure_line(release, line_published, false)?;
    }
    let kinds: Vec<Kind> = Kind::all()
        .into_iter()
        .filter(|kind| repository::document_path(*kind, root, release).exists())
        .collect();
    if kinds.is_empty() {
        return Err(KataError::Document(format!(
            "no catalog documents found for {release}"
        )));
    }

    let mut published = Vec::new();
    for kind in kinds {
        published.push(publish_one(root, release, kind, channels, force)?);
    }

    Ok(published)
}

fn fetch_document_bytes(reference: &str) -> Result<Option<Vec<u8>>> {
    let mut bytes: Option<Vec<u8>> = None;
    pull::files(
        reference,
        &Arch::Amd64,
        None,
        &koci::progress::Noop,
        |entry| {
            if entry.path == DOCUMENT_PATH {
                let mut buffer = Vec::new();
                entry.reader.read_to_end(&mut buffer)?;
                bytes = Some(buffer);
            }

            Ok(())
        },
    )
    .map_err(|error| KataError::Registry(error.to_string()))?;

    Ok(bytes)
}

fn served_document(line_reference: &str, kind: Kind, release: &str) -> Result<Option<Document>> {
    match fetch_document_bytes(line_reference)? {
        Some(bytes) => Ok(Some(from_toml(kind, &bytes, release)?)),
        None => Ok(None),
    }
}

fn ensure_channel_forward(channel: &str, served: &str, release: &str) -> Result<()> {
    let (Ok(served_version), Ok(new_version)) = (release_version(served), release_version(release))
    else {
        return Err(KataError::Gate(format!(
            "channel '{channel}' serves '{served}', which is not comparable to '{release}'"
        )));
    };

    if new_version < served_version {
        return Err(KataError::Gate(format!(
            "channel '{channel}' would move backwards: '{served}' → '{release}'; pass --force."
        )));
    }

    Ok(())
}

fn check_channel_forward(kind: Kind, release: &str, channel: &str) -> Result<()> {
    let reference = super::reference(kind.dir(), channel);
    let Some(bytes) = fetch_document_bytes(&reference)? else {
        return Ok(());
    };
    let served = from_toml_tag(kind, &bytes, channel)
        .map_err(|error| KataError::Registry(error.to_string()))?;

    ensure_channel_forward(channel, served.release(), release)
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
                     published pins never change. Compose a new line, or pass --force",
                    existing.tag, existing.digest, check.tag, check.digest
                )));
            }
            _ => {}
        }
    }

    if !served_entries.is_empty() {
        let removed: Vec<String> = served_entries.into_keys().collect();
        return Err(KataError::Gate(format!(
            "published entries would be removed: {}. Removals land in the next line, or pass --force.",
            removed.join(", ")
        )));
    }

    Ok(())
}

fn publish_one(
    root: &Path,
    release: &str,
    kind: Kind,
    channels: &[String],
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

    verify::verify_document(&document)?;

    let line_reference = super::reference(kind.dir(), release);
    let line_published = registry::manifest_exists(&line_reference)
        .map_err(|error| KataError::Registry(error.to_string()))?;

    if !force
        && line_published
        && let Some(served) = served_document(&line_reference, kind, release)?
    {
        check_monotonic(kind, &document, &served)?;
    }

    if !force {
        for channel in channels {
            check_channel_forward(kind, release, channel)?;
        }
    }

    let image = format!("{}/{}", super::registry(), kind.dir());
    let mut tags = vec![release.to_owned()];
    tags.extend(channels.iter().cloned());
    let pushed = push::files(
        &image,
        &tags,
        &Arch::Amd64,
        &[push::Entry {
            path: "catalog.toml".to_owned(),
            source: repository::document_path(kind, root, release),
        }],
    )
    .map_err(|error| KataError::Registry(error.to_string()))?;
    eprintln!("Published {image}:{release}");
    for channel in channels {
        eprintln!("Moved channel {image}:{channel} → {release}");
    }

    Ok(pushed.digest)
}

#[cfg(test)]
mod tests {
    use super::ensure_channel_forward;

    #[test]
    fn channel_moves_forward_and_stays_put() {
        // ACT / ASSERT
        ensure_channel_forward("stable", "v1.1.0", "v1.2.0").expect("forward move");
        ensure_channel_forward("stable", "v1.2.0", "v1.2.0").expect("same release");
        ensure_channel_forward("stable", "v1.1.0", "v1.2.0-beta").expect("newer prerelease");
    }

    #[test]
    fn channel_refuses_backwards_moves() {
        // ACT / ASSERT
        assert!(ensure_channel_forward("stable", "v1.2.0", "v1.1.0").is_err());
        assert!(ensure_channel_forward("stable", "v1.2.0", "v1.1.0-beta").is_err());
        assert!(ensure_channel_forward("beta", "v1.2.0", "v1.2.0-beta").is_err());
    }

    #[test]
    fn channel_refuses_incomparable_served_releases() {
        // ACT / ASSERT
        assert!(ensure_channel_forward("stable", "latest", "v1.1.0").is_err());
        assert!(ensure_channel_forward("stable", "v1.1.0", "garbage").is_err());
    }
}
