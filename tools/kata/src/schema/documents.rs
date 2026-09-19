//! Catalog document shapes.

use serde::{Deserialize, Serialize};

use crate::error;
use crate::schema::entries::{NamedEntry, SourcedEntry};
use crate::schema::kinds::Kind;

/// Path of the single catalog document inside its OCI image.
pub const DOCUMENT_PATH: &str = "catalog.toml";

/// Core catalog document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreDocument {
    /// Schema version.
    pub api_version: String,
    /// Release line this catalog belongs to; must match its tag.
    pub release: String,
    /// Curated kernels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kernels: Vec<SourcedEntry>,
    /// Frozen build machinery seeded at release.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stub: Option<SourcedEntry>,
    /// Frozen build machinery seeded at release.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installer: Option<SourcedEntry>,
}

/// Overlay catalog document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayDocument {
    /// Schema version.
    pub api_version: String,
    /// Release line this catalog belongs to; must match its tag.
    pub release: String,
    /// Board overlays; one payload image may serve several names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overlays: Vec<NamedEntry>,
}

/// Extension catalog document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionDocument {
    /// Schema version.
    pub api_version: String,
    /// Release line this catalog belongs to; must match its tag.
    pub release: String,
    /// System extensions; kernel-ABI-coupled and seeded per release line.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<NamedEntry>,
}

/// A loaded catalog document of any kind.
#[derive(Debug, Clone)]
pub enum Document {
    /// Core catalog: kernels plus the frozen stub and installer.
    Core(CoreDocument),
    /// Overlay catalog.
    Overlays(OverlayDocument),
    /// Extension catalog.
    Extensions(ExtensionDocument),
}

impl Document {
    /// Returns the curated kernel entry matching `source`.
    ///
    /// # Errors
    ///
    /// Returns an error when the document is not a core catalog or the source
    /// is not curated.
    pub fn kernel(&self, source: &str) -> error::DocumentResult<&SourcedEntry> {
        let Self::Core(ref core) = *self else {
            return Err(mismatch("kernel"));
        };

        core.kernels
            .iter()
            .find(|entry| entry.source == source)
            .ok_or_else(|| {
                error::DocumentError::Document(format!(
                    "core catalog does not contain kernel source '{source}'"
                ))
            })
    }

    /// Returns the frozen stub entry.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry is not seeded.
    pub fn stub(&self) -> error::DocumentResult<&SourcedEntry> {
        frozen(self, "stub", |core| core.stub.as_ref())
    }

    /// Returns the frozen installer entry.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry is not seeded.
    pub fn installer(&self) -> error::DocumentResult<&SourcedEntry> {
        frozen(self, "installer", |core| core.installer.as_ref())
    }

    /// Returns the named overlay or extension entry.
    #[must_use]
    pub fn named(&self, name: &str) -> Option<&NamedEntry> {
        let entries = match *self {
            Self::Overlays(ref overlays) => &overlays.overlays,
            Self::Extensions(ref extensions) => &extensions.extensions,
            Self::Core(_) => return None,
        };

        entries.iter().find(|entry| entry.name == name)
    }

    /// Kind of this document.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        match *self {
            Self::Core(_) => Kind::Core,
            Self::Overlays(_) => Kind::Overlays,
            Self::Extensions(_) => Kind::Extensions,
        }
    }

    /// Schema version declared by this document.
    #[must_use]
    pub fn api_version(&self) -> &str {
        match *self {
            Self::Core(ref document) => &document.api_version,
            Self::Overlays(ref document) => &document.api_version,
            Self::Extensions(ref document) => &document.api_version,
        }
    }

    /// Release line of this document.
    #[must_use]
    pub fn release(&self) -> &str {
        match *self {
            Self::Core(ref document) => &document.release,
            Self::Overlays(ref document) => &document.release,
            Self::Extensions(ref document) => &document.release,
        }
    }

    /// Rewrite the release line (when seeding a new line from a previous one).
    pub fn set_release(&mut self, release: String) {
        match *self {
            Self::Core(ref mut document) => document.release = release,
            Self::Overlays(ref mut document) => document.release = release,
            Self::Extensions(ref mut document) => document.release = release,
        }
    }
}

fn frozen<'a>(
    document: &'a Document,
    role: &str,
    slot: fn(&'a CoreDocument) -> Option<&'a SourcedEntry>,
) -> error::DocumentResult<&'a SourcedEntry> {
    let Document::Core(ref core) = *document else {
        return Err(mismatch(role));
    };

    slot(core).ok_or_else(|| {
        error::DocumentError::Document(format!("core catalog is missing the {role} entry"))
    })
}

fn mismatch(role: &str) -> error::DocumentError {
    error::DocumentError::Document(format!(
        "'{role}' lookup requires the core catalog document"
    ))
}
