//! Public source resolution API.

mod engine;

use koci::arch;
use koci::arch::Arch;

use crate::error::Result;
use crate::profile::Profile;
use crate::request::{Platform, Request};

/// Pipeline configuration shared across build and install paths.
#[derive(Debug, Clone)]
pub struct Config {
    /// OCI image registry and installer repository.
    pub sources: Sources,
}

/// Source configuration for the build pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sources {
    /// OCI registry hostname.
    pub registry: String,
    /// Installer repository path within the registry.
    pub installer: String,
}

/// A reference to an extension source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionRef {
    name: String,
    source: String,
}

impl ExtensionRef {
    #[must_use]
    pub(crate) fn new(name: String, source: String) -> Self {
        Self { name, source }
    }

    /// Returns the canonical logical extension name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the versioned OCI reference for this extension.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// An overlay source resolved from the profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlaySource {
    /// Overlay name inside the OCI image.
    pub name: String,
    /// Logical overlay image name.
    pub image: String,
    /// Versioned OCI reference for the overlay image.
    pub source: String,
    /// Target architecture of the overlay.
    pub arch: Arch,
}

impl OverlaySource {
    #[must_use]
    pub(crate) fn new(name: String, image: String, source: String, arch: Arch) -> Self {
        Self {
            name,
            image,
            source,
            arch,
        }
    }

    /// Returns the selected overlay name inside the OCI image.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the logical overlay image name.
    #[must_use]
    pub fn image(&self) -> &str {
        &self.image
    }

    /// Returns the versioned OCI reference for this overlay image.
    #[must_use]
    pub fn source_ref(&self) -> &str {
        &self.source
    }
}

/// Canonical build plan produced from a request and profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildPlan {
    platform: Platform,
    version: String,
    arch: Arch,
    extensions: Vec<ExtensionRef>,
    overlay: Option<OverlaySource>,
    installer: String,
}

impl BuildPlan {
    #[must_use]
    pub(crate) fn new(
        platform: Platform,
        version: String,
        arch: Arch,
        extensions: Vec<ExtensionRef>,
        overlay: Option<OverlaySource>,
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
    pub fn extensions(&self) -> &[ExtensionRef] {
        &self.extensions
    }

    /// Returns the resolved overlay input when present.
    #[must_use]
    pub fn overlay(&self) -> Option<&OverlaySource> {
        self.overlay.as_ref()
    }

    /// Returns the versioned installer OCI reference.
    #[must_use]
    pub fn installer(&self) -> &str {
        &self.installer
    }
}

/// Resolves a profile and request into a build plan with versioned OCI references.
///
/// # Errors
///
/// Returns an error when the profile references an unknown source input.
pub fn profile(request: &Request, profile: &Profile, sources: &Sources) -> Result<BuildPlan> {
    let host = arch::host();
    let arch = request.arch.unwrap_or(host);

    engine::resolve(&request.version, request.platform, arch, profile, sources)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_extension_accessors() {
        // ARRANGE
        let ext = ExtensionRef::new("muak-os/qemu".into(), "ghcr.io/muak-os/qemu:v1.0.0".into());

        // ACT / ASSERT
        assert_eq!(ext.name(), "muak-os/qemu");
        assert_eq!(ext.source(), "ghcr.io/muak-os/qemu:v1.0.0");
    }

    #[test]
    fn resolved_overlay_accessors() {
        // ARRANGE
        let ov = OverlaySource::new(
            "rpi_generic".into(),
            "muak-os/sbc-raspberrypi".into(),
            "ghcr.io/muak-os/sbc-raspberrypi:v1.0.0".into(),
            Arch::Amd64,
        );

        // ACT / ASSERT
        assert_eq!(ov.name(), "rpi_generic");
        assert_eq!(ov.image(), "muak-os/sbc-raspberrypi");
        assert_eq!(ov.source_ref(), "ghcr.io/muak-os/sbc-raspberrypi:v1.0.0");
    }

    #[test]
    fn resolved_profile_accessors() {
        // ARRANGE
        let ext = ExtensionRef::new("muak-os/qemu".into(), "ghcr.io/muak-os/qemu:v1.0.0".into());

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
