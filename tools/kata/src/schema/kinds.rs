//! Per-kind catalog identities.

use crate::error;

/// Schema version of core catalog documents.
pub const CORE_API_VERSION: &str = "muak.dev/catalog/core/v1";
/// Schema version of overlay catalog documents.
pub const OVERLAYS_API_VERSION: &str = "muak.dev/catalog/overlays/v1";
/// Schema version of extension catalog documents.
pub const EXTENSIONS_API_VERSION: &str = "muak.dev/catalog/extensions/v1";

/// Which of the three per-kind catalog documents a path or command names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Kernel, stub, and installer pointers.
    Core,
    /// Board boot-asset overlays.
    Overlays,
    /// System extensions (initramfs layers).
    Extensions,
}

impl Kind {
    /// Parses a kind name as accepted by `verify` and `publish`.
    ///
    /// # Errors
    ///
    /// Returns an error for names other than `core`, `overlays`, or
    /// `extensions`.
    pub fn parse(name: &str) -> error::DocumentResult<Self> {
        match name {
            "core" => Ok(Self::Core),
            "overlays" => Ok(Self::Overlays),
            "extensions" => Ok(Self::Extensions),
            other => Err(error::DocumentError::Document(format!(
                "unknown catalog kind '{other}' (expected core, overlays, or extensions)"
            ))),
        }
    }

    /// Repository name of this kind's published catalog image.
    #[must_use]
    pub const fn repository(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Overlays => "overlays",
            Self::Extensions => "extensions",
        }
    }

    /// Directory holding this kind's local catalog documents.
    #[must_use]
    pub const fn dir(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Overlays => "overlays",
            Self::Extensions => "extensions",
        }
    }

    /// Expected `api_version` of this kind's documents.
    #[must_use]
    pub const fn api_version(self) -> &'static str {
        match self {
            Self::Core => CORE_API_VERSION,
            Self::Overlays => OVERLAYS_API_VERSION,
            Self::Extensions => EXTENSIONS_API_VERSION,
        }
    }

    /// Every kind, in publication order.
    #[must_use]
    pub const fn all() -> [Self; 3] {
        [Self::Core, Self::Overlays, Self::Extensions]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_parses_catalog_kind_names() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(Kind::parse("core").expect("parse core"), Kind::Core);
        assert_eq!(
            Kind::parse("overlays").expect("parse overlays"),
            Kind::Overlays
        );
        assert_eq!(
            Kind::parse("extensions").expect("parse extensions"),
            Kind::Extensions
        );
        Kind::parse("kernel").unwrap_err();
    }
}
