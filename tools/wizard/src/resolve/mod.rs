//! Public source resolution API.

mod engine;

use koci::arch;
use koci::arch::Arch;

use crate::config;
use crate::error::Result;
use crate::profile::Profile;
use crate::request::{Platform, Request};
use crate::source::extension::Extension;
use crate::source::overlay::Overlay;

/// Canonical build plan produced from a request and profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildPlan {
    platform: Platform,
    version: String,
    arch: Arch,
    extensions: Vec<Extension>,
    overlay: Option<Overlay>,
    installer: String,
}

impl BuildPlan {
    #[must_use]
    pub(crate) fn new(
        platform: Platform,
        version: String,
        arch: Arch,
        extensions: Vec<Extension>,
        overlay: Option<Overlay>,
        installer: String,
    ) -> Self {
        Self {
            platform,
            version,
            arch,
            extensions,
            overlay,
            installer,
        }
    }

    /// Returns the resolved platform for the build.
    #[must_use]
    pub const fn platform(&self) -> Platform {
        self.platform
    }

    /// Returns the requested Muak version.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns the requested target architecture.
    #[must_use]
    pub const fn arch(&self) -> Arch {
        self.arch
    }

    /// Returns the resolved extension inputs in canonical order.
    #[must_use]
    pub fn extensions(&self) -> &[Extension] {
        &self.extensions
    }

    /// Returns the resolved overlay input when present.
    #[must_use]
    pub fn overlay(&self) -> Option<&Overlay> {
        self.overlay.as_ref()
    }

    /// Returns the versioned installer OCI reference.
    #[must_use]
    pub fn installer(&self) -> &str {
        &self.installer
    }
}

/// Resolves a request and profile into a build plan with versioned OCI references.
///
/// # Errors
///
/// Returns an error when the profile references an unknown source input or
/// when the global configuration has not been set.
pub fn plan(request: &Request, profile: &Profile) -> Result<BuildPlan> {
    let sources = &config::config()?.sources;
    let host = arch::host();
    let arch = request.target_arch().unwrap_or(host);

    engine::resolve(
        request.version(),
        request.platform(),
        arch,
        profile,
        sources,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_profile_accessors() {
        // ARRANGE
        let ext = Extension::new("muak-os/qemu".into(), "ghcr.io/muak-os/qemu:v1.0.0".into());

        // ACT
        let bp = BuildPlan::new(
            Platform::Metal,
            "v1.0.0-beta".into(),
            Arch::Amd64,
            vec![ext],
            None,
            "ghcr.io/muak-os/installer:v1.0.0-beta".into(),
        );

        // ASSERT
        assert_eq!(bp.platform(), Platform::Metal);
        assert_eq!(bp.version(), "v1.0.0-beta");
        assert_eq!(bp.arch(), Arch::Amd64);
        assert_eq!(bp.extensions().len(), 1);
        assert!(bp.overlay().is_none());
        assert_eq!(bp.installer(), "ghcr.io/muak-os/installer:v1.0.0-beta");
    }
}
