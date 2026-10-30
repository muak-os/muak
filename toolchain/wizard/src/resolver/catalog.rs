//! Catalog image fetching and pin construction.

use kata::schema::documents::{DOCUMENT_PATH, Document};
use kata::schema::kinds::Kind;
use kata::schema::parse::from_toml_tag;
use koci::arch::Arch;

use crate::error::{Result, WizardError};

pub(super) struct Fetched {
    pub(super) document: Document,
    pub(super) digest: String,
}

pub(super) fn fetch(kind: Kind, release: &str, registry: &str) -> Result<Document> {
    let document = fetch_tag(kind, release, registry)?.document;
    if document.release() != release {
        return Err(WizardError::SourceResolution(format!(
            "catalog release '{}' does not match '{release}'",
            document.release()
        )));
    }

    Ok(document)
}

pub(super) fn fetch_tag(kind: Kind, tag: &str, registry: &str) -> Result<Fetched> {
    let reference = format!("{registry}/{}:{tag}", kind.dir());
    let mut document: Option<Vec<u8>> = None;
    let resolution = koci::pull::files(
        &reference,
        &Arch::Amd64,
        None,
        &koci::progress::Noop,
        |entry| {
            if entry.path == DOCUMENT_PATH {
                let mut buffer = Vec::new();
                entry.reader.read_to_end(&mut buffer)?;
                document = Some(buffer);
            }

            Ok(())
        },
    )
    .map_err(|error| {
        WizardError::SourceResolution(format!("fetch catalog image {reference}: {error}"))
    })?;
    let bytes = document.ok_or_else(|| {
        WizardError::SourceResolution(format!(
            "catalog image {reference} does not contain {DOCUMENT_PATH}",
        ))
    })?;
    let document = from_toml_tag(kind, &bytes, tag).map_err(WizardError::from)?;

    Ok(Fetched {
        document,
        digest: resolution.digest,
    })
}

pub(super) fn pinned_reference(registry: &str, repository: &str, digest: &str) -> String {
    format!("{registry}/{repository}@{digest}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_reference_is_registry_scoped_and_digest_addressed() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(
            pinned_reference("localhost:5000", "linux", "sha256:1111"),
            "localhost:5000/linux@sha256:1111"
        );
    }
}
