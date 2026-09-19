//! Flattened entry views across catalog documents.

use crate::error;
use crate::schema::documents::Document;
use crate::schema::entries::{NamedEntry, SourcedEntry};
use crate::schema::kinds::Kind;

/// Flattened role of a catalog entry within the resolution identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// Curated kernel image.
    Kernel,
    /// Frozen EFI stub image.
    Stub,
    /// Frozen installer environment image.
    Installer,
    /// Board overlay image.
    Overlay,
    /// System extension image.
    Extension,
}

impl Role {
    /// Parses an entry-kind name as accepted by `kata add`.
    ///
    /// # Errors
    ///
    /// Returns an error for any other name.
    pub fn parse(name: &str) -> error::DocumentResult<Self> {
        match name {
            "kernel" => Ok(Self::Kernel),
            "stub" => Ok(Self::Stub),
            "installer" => Ok(Self::Installer),
            "overlays" => Ok(Self::Overlay),
            "extensions" => Ok(Self::Extension),
            other => Err(error::DocumentError::Document(format!(
                "unknown entry kind '{other}' (expected kernel, stub, installer, overlays, or extensions)"
            ))),
        }
    }

    /// Role name in a resolution identity record, frozen by the recipe.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Kernel => "kernel",
            Self::Stub => "stub",
            Self::Installer => "installer",
            Self::Overlay => "overlay",
            Self::Extension => "extension",
        }
    }

    /// Catalog document holding this role's entries.
    #[must_use]
    pub const fn catalog_kind(self) -> Kind {
        match self {
            Self::Kernel | Self::Stub | Self::Installer => Kind::Core,
            Self::Overlay => Kind::Overlays,
            Self::Extension => Kind::Extensions,
        }
    }

    /// Whether entries of this role are identified by a `name`.
    #[must_use]
    pub const fn requires_name(self) -> bool {
        matches!(self, Self::Overlay | Self::Extension)
    }
}

/// One catalog entry viewed uniformly, borrowed from its document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryRef<'a> {
    /// Role the entry plays in a resolution.
    pub role: Role,
    /// Logical identity: `source` (core entries) or `name` (overlays, extensions).
    pub identity: &'a str,
    /// Multi-arch index digest pinning the entry image.
    pub digest: &'a str,
}

/// Every entry of `document` as a uniform borrowed view, in schema order.
#[must_use]
pub fn entries(document: &Document) -> Vec<EntryRef<'_>> {
    match *document {
        Document::Core(ref core) => core
            .kernels
            .iter()
            .map(|entry| sourced(Role::Kernel, entry))
            .chain(core.stub.iter().map(|entry| sourced(Role::Stub, entry)))
            .chain(
                core.installer
                    .iter()
                    .map(|entry| sourced(Role::Installer, entry)),
            )
            .collect(),
        Document::Overlays(ref overlays) => overlays
            .overlays
            .iter()
            .map(|entry| named(Role::Overlay, entry))
            .collect(),
        Document::Extensions(ref extensions) => extensions
            .extensions
            .iter()
            .map(|entry| named(Role::Extension, entry))
            .collect(),
    }
}

/// Views a sourced core entry under `role`.
#[must_use]
pub fn sourced(role: Role, entry: &SourcedEntry) -> EntryRef<'_> {
    EntryRef {
        role,
        identity: &entry.source,
        digest: &entry.digest,
    }
}

/// Views a named overlay or extension entry under `role`.
#[must_use]
pub fn named(role: Role, entry: &NamedEntry) -> EntryRef<'_> {
    EntryRef {
        role,
        identity: &entry.name,
        digest: &entry.digest,
    }
}

#[cfg(test)]
mod tests {
    use crate::schema::documents::{CoreDocument, Document, ExtensionDocument, OverlayDocument};
    use crate::schema::entries::{NamedEntry, SourcedEntry};
    use crate::schema::kinds::{
        CORE_API_VERSION, EXTENSIONS_API_VERSION, Kind, OVERLAYS_API_VERSION,
    };
    use crate::schema::view::{EntryRef, Role, entries};

    fn sourced(source: &str, digest: &str) -> SourcedEntry {
        SourcedEntry {
            source: source.to_owned(),
            repository: "repo".to_owned(),
            tag: "v1".to_owned(),
            digest: digest.to_owned(),
        }
    }

    fn named(name: &str, digest: &str) -> NamedEntry {
        NamedEntry {
            name: name.to_owned(),
            source: "muak-os/fixtures".to_owned(),
            repository: "repo".to_owned(),
            tag: "v1".to_owned(),
            digest: digest.to_owned(),
        }
    }

    fn core_document(kernels: Vec<SourcedEntry>) -> Document {
        Document::Core(CoreDocument {
            api_version: CORE_API_VERSION.to_owned(),
            release: "v1.2.3".to_owned(),
            kernels,
            stub: Some(sourced("muak-os/stub", "sha256:s")),
            installer: Some(sourced("muak-os/installer", "sha256:i")),
        })
    }

    fn overlay_document(entries: Vec<NamedEntry>) -> Document {
        Document::Overlays(OverlayDocument {
            api_version: OVERLAYS_API_VERSION.to_owned(),
            release: "v1.2.3".to_owned(),
            overlays: entries,
        })
    }

    fn extension_document(entries: Vec<NamedEntry>) -> Document {
        Document::Extensions(ExtensionDocument {
            api_version: EXTENSIONS_API_VERSION.to_owned(),
            release: "v1.2.3".to_owned(),
            extensions: entries,
        })
    }

    #[test]
    fn core_document_yields_kernels_then_frozen_entries() {
        // ARRANGE
        let document = core_document(vec![
            sourced("muak-os/linux-a", "sha256:1"),
            sourced("muak-os/linux-b", "sha256:2"),
        ]);

        // ACT
        let entries = entries(&document);
        let flat: Vec<(Role, &str)> = entries
            .iter()
            .map(|entry| (entry.role, entry.identity))
            .collect();

        // ASSERT
        assert_eq!(
            flat,
            vec![
                (Role::Kernel, "muak-os/linux-a"),
                (Role::Kernel, "muak-os/linux-b"),
                (Role::Stub, "muak-os/stub"),
                (Role::Installer, "muak-os/installer"),
            ]
        );
        let first = entries.first().expect("core documents yield entries");
        assert_eq!(first.digest, "sha256:1");
    }

    #[test]
    fn named_documents_use_the_name_as_identity() {
        // ARRANGE
        let overlays = overlay_document(vec![named("rpi_generic", "sha256:o")]);
        let extensions = extension_document(vec![named("qemu-guest", "sha256:e")]);

        // ACT / ASSERT
        assert_eq!(
            entries(&overlays),
            vec![EntryRef {
                role: Role::Overlay,
                identity: "rpi_generic",
                digest: "sha256:o",
            }]
        );
        assert_eq!(
            entries(&extensions),
            vec![EntryRef {
                role: Role::Extension,
                identity: "qemu-guest",
                digest: "sha256:e",
            }]
        );
    }

    #[test]
    fn role_parses_entry_kind_names() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(Role::parse("kernel").expect("parse kernel"), Role::Kernel);
        assert_eq!(Role::parse("stub").expect("parse stub"), Role::Stub);
        assert_eq!(
            Role::parse("installer").expect("parse installer"),
            Role::Installer
        );
        assert_eq!(
            Role::parse("overlays").expect("parse overlays"),
            Role::Overlay
        );
        assert_eq!(
            Role::parse("extensions").expect("parse extensions"),
            Role::Extension
        );
        Role::parse("board").unwrap_err();
    }

    #[test]
    fn roles_map_to_catalog_kinds_and_name_requirement() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(Role::Kernel.catalog_kind(), Kind::Core);
        assert_eq!(Role::Overlay.catalog_kind(), Kind::Overlays);
        assert_eq!(Role::Extension.catalog_kind(), Kind::Extensions);
        assert!(Role::Overlay.requires_name());
        assert!(Role::Extension.requires_name());
        assert!(!Role::Kernel.requires_name());
        assert!(!Role::Stub.requires_name());
        assert!(!Role::Installer.requires_name());
    }

    #[test]
    fn role_names_and_order_match_the_frozen_recipe() {
        // ARRANGE
        let roles = [
            Role::Kernel,
            Role::Stub,
            Role::Installer,
            Role::Overlay,
            Role::Extension,
        ];

        // ACT
        let names: Vec<&str> = roles.iter().map(|role| role.as_str()).collect();
        let mut sorted = [Role::Extension, Role::Kernel];
        sorted.sort();

        // ASSERT
        assert_eq!(
            names,
            ["kernel", "stub", "installer", "overlay", "extension"]
        );
        assert_eq!(sorted, [Role::Kernel, Role::Extension]);
    }
}
