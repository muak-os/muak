//! Resolved source model types used by the build pipeline.

use koci::arch::Arch;

use crate::request::Platform;

/// Resolved extension source with stable naming for `ramune`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedExtension {
    name: String,
    source: String,
}

impl ResolvedExtension {
    /// Creates a resolved extension from its canonical identifiers.
    #[must_use]
    pub fn new(name: String, source: String) -> Self {
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

/// Resolved overlay source selected by the profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedOverlay {
    name: String,
    image: String,
    source: String,
}

impl ResolvedOverlay {
    /// Creates a resolved overlay from its canonical identifiers.
    #[must_use]
    pub fn new(name: String, image: String, source: String) -> Self {
        Self {
            name,
            image,
            source,
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

/// Canonical internal build profile resolved from a request and profile spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBuildProfile {
    platform: Platform,
    version: String,
    arch: Arch,
    extensions: Vec<ResolvedExtension>,
    overlay: Option<ResolvedOverlay>,
    installer: String,
}

impl ResolvedBuildProfile {
    /// Creates a resolved build profile from all resolved source inputs.
    #[must_use]
    pub fn new(
        platform: Platform,
        version: String,
        arch: Arch,
        extensions: Vec<ResolvedExtension>,
        overlay: Option<ResolvedOverlay>,
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
    pub fn extensions(&self) -> &[ResolvedExtension] {
        &self.extensions
    }

    /// Returns the resolved overlay input when present.
    #[must_use]
    pub fn overlay(&self) -> Option<&ResolvedOverlay> {
        self.overlay.as_ref()
    }

    /// Returns the versioned installer OCI reference.
    #[must_use]
    pub fn installer(&self) -> &str {
        &self.installer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_extension_accessors() {
        // ARRANGE
        let ext =
            ResolvedExtension::new("muak-os/qemu".into(), "ghcr.io/muak-os/qemu:v1.0.0".into());

        // ACT / ASSERT
        assert_eq!(ext.name(), "muak-os/qemu");
        assert_eq!(ext.source(), "ghcr.io/muak-os/qemu:v1.0.0");
    }

    #[test]
    fn resolved_overlay_accessors() {
        // ARRANGE
        let ov = ResolvedOverlay::new(
            "rpi_generic".into(),
            "muak-os/sbc-raspberrypi".into(),
            "ghcr.io/muak-os/sbc-raspberrypi:v1.0.0".into(),
        );

        // ACT / ASSERT
        assert_eq!(ov.name(), "rpi_generic");
        assert_eq!(ov.image(), "muak-os/sbc-raspberrypi");
        assert_eq!(ov.source_ref(), "ghcr.io/muak-os/sbc-raspberrypi:v1.0.0");
    }

    #[test]
    fn resolved_build_profile_accessors() {
        // ARRANGE
        let ext =
            ResolvedExtension::new("muak-os/qemu".into(), "ghcr.io/muak-os/qemu:v1.0.0".into());
        let bp = ResolvedBuildProfile::new(
            Platform::Metal,
            "v1.0.0-beta".into(),
            Arch::Amd64,
            vec![ext],
            None,
            "ghcr.io/muak-os/installer:v1.0.0-beta".into(),
        );

        // ACT / ASSERT
        assert_eq!(bp.platform(), Platform::Metal);
        assert_eq!(bp.version(), "v1.0.0-beta");
        assert_eq!(bp.arch(), Arch::Amd64);
        assert_eq!(bp.extensions().len(), 1);
        assert!(bp.overlay().is_none());
        assert_eq!(
            bp.installer(),
            "ghcr.io/muak-os/installer:v1.0.0-beta"
        );
    }
}
