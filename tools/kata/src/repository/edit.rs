//! Mutations of loaded catalog documents, enforcing the mutation policy.

use crate::error::{KataError, Result};
use crate::schema::documents::Document;
use crate::schema::entries::{NamedEntry, SourcedEntry};
use crate::schema::kinds::Kind;

/// Insert or replace a kernel entry.
///
/// # Errors
///
/// Returns an error when the document is not a core catalog.
pub fn set_kernel(document: &mut Document, entry: SourcedEntry) -> Result<()> {
    let Document::Core(ref mut core) = *document else {
        return Err(wrong_kind("kernel", document.kind()));
    };

    match core
        .kernels
        .iter_mut()
        .find(|existing| existing.source == entry.source)
    {
        Some(existing) => {
            if existing.digest != entry.digest {
                eprintln!(
                    "Replaced kernel '{}' digest {} with {}",
                    entry.source, existing.digest, entry.digest
                );
            }
            *existing = entry;
        }
        None => core.kernels.push(entry),
    }

    Ok(())
}

/// Insert or replace a named overlay or extension entry.
///
/// # Errors
///
/// Returns an error when the document is neither an overlay nor an extension catalog.
pub fn set_named(document: &mut Document, entry: NamedEntry) -> Result<()> {
    match *document {
        Document::Overlays(ref mut overlays) => {
            replace_named(&mut overlays.overlays, entry, "overlay");
        }
        Document::Extensions(ref mut extensions) => {
            replace_named(&mut extensions.extensions, entry, "extension");
        }
        Document::Core(_) => return Err(wrong_kind("named", Kind::Core)),
    }

    Ok(())
}

/// Seed the frozen stub entry rejecting a differing replacement.
///
/// # Errors
///
/// Returns an error when the document is not a core catalog or the stub differs from the frozen one.
pub fn set_stub(document: &mut Document, entry: SourcedEntry) -> Result<()> {
    let Document::Core(ref mut core) = *document else {
        return Err(wrong_kind("stub", document.kind()));
    };

    replace_frozen(&mut core.stub, entry, "stub")
}

/// Seed the frozen installer entry rejecting a differing replacement.
///
/// # Errors
///
/// Returns an error when the document is not a core catalog or the installer differs from the frozen one.
pub fn set_installer(document: &mut Document, entry: SourcedEntry) -> Result<()> {
    let Document::Core(ref mut core) = *document else {
        return Err(wrong_kind("installer", document.kind()));
    };

    replace_frozen(&mut core.installer, entry, "installer")
}

fn replace_named(entries: &mut Vec<NamedEntry>, entry: NamedEntry, role: &str) {
    match entries
        .iter_mut()
        .find(|existing| existing.name == entry.name)
    {
        Some(existing) => {
            if existing.digest != entry.digest {
                eprintln!(
                    "Replaced {role} '{}' digest {} with {}",
                    entry.name, existing.digest, entry.digest
                );
            }
            *existing = entry;
        }
        None => entries.push(entry),
    }
}

fn replace_frozen(slot: &mut Option<SourcedEntry>, entry: SourcedEntry, role: &str) -> Result<()> {
    match *slot {
        Some(ref existing) if *existing != entry => Err(KataError::Frozen(format!(
            "{role} is frozen for the release line (existing digest {}, new {})",
            existing.digest, entry.digest
        ))),
        Some(_) => Ok(()),
        None => {
            *slot = Some(entry);

            Ok(())
        }
    }
}

fn wrong_kind(role: &str, kind: Kind) -> KataError {
    KataError::Document(format!(
        "'{role}' entry does not belong to a {} document",
        kind.dir()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::documents::CoreDocument;
    use crate::schema::kinds::CORE_API_VERSION;

    fn core_document() -> Document {
        Document::Core(CoreDocument {
            api_version: CORE_API_VERSION.to_owned(),
            release: "v1.2.3".to_owned(),
            kernels: Vec::new(),
            stub: None,
            installer: None,
        })
    }

    fn sourced(digest: &str) -> SourcedEntry {
        SourcedEntry {
            source: "muak-os/linux".to_owned(),
            repository: "kernel".to_owned(),
            tag: "v6.12.4-muak1".to_owned(),
            digest: digest.to_owned(),
        }
    }

    #[test]
    fn set_kernel_adds_and_replaces_by_source() {
        // ARRANGE
        let mut document = core_document();

        // ACT
        set_kernel(&mut document, sourced("sha256:aaa")).expect("add kernel");
        set_kernel(&mut document, sourced("sha256:bbb")).expect("replace kernel");

        // ASSERT
        let Document::Core(ref core) = document else {
            panic!("core document expected");
        };
        assert_eq!(core.kernels.len(), 1);
        assert_eq!(core.kernels.first().expect("kernel").digest, "sha256:bbb");
    }

    #[test]
    fn set_stub_rejects_differing_replacements() {
        // ARRANGE
        let mut document = core_document();
        set_stub(&mut document, sourced("sha256:aaa")).expect("seed stub");

        // ACT
        let error = set_stub(&mut document, sourced("sha256:bbb"))
            .expect_err("frozen entry should reject changes");

        // ASSERT
        assert!(matches!(error, KataError::Frozen(_)));
        set_stub(&mut document, sourced("sha256:aaa")).expect("identical seed is idempotent");
    }

    #[test]
    fn set_named_rejects_core_documents() {
        // ARRANGE
        let mut document = core_document();
        let entry = NamedEntry {
            name: "rpi_generic".to_owned(),
            source: "muak-os/sbc-raspberrypi".to_owned(),
            repository: "sbc/raspberrypi".to_owned(),
            tag: "v0.4.0".to_owned(),
            digest: "sha256:aaa".to_owned(),
        };

        // ACT / ASSERT
        let error = set_named(&mut document, entry).expect_err("core document should reject");
        assert!(matches!(error, KataError::Document(_)));
    }
}
