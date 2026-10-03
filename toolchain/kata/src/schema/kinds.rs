//! Per-kind catalog identities.

/// Schema version of core catalog documents.
pub const CORE_API_VERSION: &str = "muak.dev/catalog/core/v1";
/// Schema version of overlay catalog documents.
pub const OVERLAYS_API_VERSION: &str = "muak.dev/catalog/overlays/v1";
/// Schema version of extension catalog documents.
pub const EXTENSIONS_API_VERSION: &str = "muak.dev/catalog/extensions/v1";

/// Which of the three per-kind catalog documents a path names.
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
    fn kinds_expose_the_expected_dirs_in_order() {
        // ARRANGE / ACT
        let dirs: Vec<&str> = Kind::all().iter().map(|kind| kind.dir()).collect();

        // ASSERT
        assert_eq!(dirs, ["core", "overlays", "extensions"]);
    }
}
