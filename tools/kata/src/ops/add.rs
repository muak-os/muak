//! Adding or updating a pinned entry after resolving its registry digest.

use std::path::{Path, PathBuf};

use koci::registry;

use crate::error::{KataError, Result};
use crate::repository;
use crate::schema::documents::{CoreDocument, Document, ExtensionDocument, OverlayDocument};
use crate::schema::entries::{NamedEntry, SourcedEntry};
use crate::schema::kinds::{CORE_API_VERSION, EXTENSIONS_API_VERSION, Kind, OVERLAYS_API_VERSION};
use crate::schema::parse::validate_release;
use crate::schema::view::Role;

/// One `kata add` request.
#[derive(Debug, Clone)]
pub struct Input {
    /// Role of the entry to pin.
    pub role: Role,
    /// Release line to write into.
    pub release: String,
    /// Logical `name` for overlays and extensions.
    pub name: Option<String>,
    /// Logical `source` of the payload repository.
    pub source: String,
    /// Repository path relative to the registry prefix.
    pub repository: String,
    /// Payload tag to resolve.
    pub tag: String,
    /// Digest claimed by the announcing payload, when dispatched.
    pub claimed_digest: Option<String>,
    /// Registry prefix for digest resolution.
    pub registry: String,
}

/// Resolve the entry digest from the registry and write it into the document.
///
/// # Errors
///
/// Returns an error when the kind and name disagree, the claimed digest does
/// not match the registry, or the document cannot be written.
pub fn run(root: &Path, input: &Input) -> Result<PathBuf> {
    validate_release(&input.release)?;
    if input.role.requires_name() != input.name.is_some() {
        return Err(KataError::Document(
            "--name is required for overlays and extensions and invalid otherwise".to_owned(),
        ));
    }

    let reference = super::reference(&input.registry, &input.repository, &input.tag);
    let digest = registry::manifest_digest(&reference)
        .map_err(|error| KataError::Registry(error.to_string()))?;
    if let Some(ref claimed) = input.claimed_digest
        && *claimed != digest
    {
        return Err(KataError::Registry(format!(
            "claimed digest {claimed} does not match registry digest {digest} for {reference}"
        )));
    }

    let kind = input.role.catalog_kind();
    let mut document = if repository::document_path(kind, root, &input.release).exists() {
        repository::load(kind, root, &input.release)?
    } else {
        fresh_document(kind, &input.release)
    };

    let sourced = SourcedEntry {
        source: input.source.clone(),
        repository: input.repository.clone(),
        tag: input.tag.clone(),
        digest: digest.clone(),
    };

    match input.role {
        Role::Kernel => repository::edit::set_kernel(&mut document, sourced)?,
        Role::Stub => repository::edit::set_stub(&mut document, sourced)?,
        Role::Installer => repository::edit::set_installer(&mut document, sourced)?,
        Role::Overlay | Role::Extension => {
            let Some(name) = input.name.as_deref() else {
                return Err(KataError::Document(
                    "--name is required for overlays and extensions".to_owned(),
                ));
            };

            repository::edit::set_named(
                &mut document,
                NamedEntry {
                    name: name.to_owned(),
                    source: input.source.clone(),
                    repository: input.repository.clone(),
                    tag: input.tag.clone(),
                    digest: digest.clone(),
                },
            )?;
        }
    }

    repository::write(&document, root, &input.release)
}

fn fresh_document(kind: Kind, release: &str) -> Document {
    match kind {
        Kind::Core => Document::Core(CoreDocument {
            api_version: CORE_API_VERSION.to_owned(),
            release: release.to_owned(),
            kernels: Vec::new(),
            stub: None,
            installer: None,
        }),
        Kind::Overlays => Document::Overlays(OverlayDocument {
            api_version: OVERLAYS_API_VERSION.to_owned(),
            release: release.to_owned(),
            overlays: Vec::new(),
        }),
        Kind::Extensions => Document::Extensions(ExtensionDocument {
            api_version: EXTENSIONS_API_VERSION.to_owned(),
            release: release.to_owned(),
            extensions: Vec::new(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_document_carries_schema_version_and_release() {
        // ARRANGE / ACT
        let document = fresh_document(Kind::Core, "v1.2.3");

        // ASSERT
        assert_eq!(document.kind(), Kind::Core);
        assert_eq!(document.api_version(), Kind::Core.api_version());
        assert_eq!(document.release(), "v1.2.3");
    }
}
