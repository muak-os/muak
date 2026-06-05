//! OCI/source resolution implementation details.

use koci::arch::Arch;

use crate::catalog::{is_official_extension, resolve_extension_name};
use crate::error::{ImagerError, Result};
use crate::profile::Profile;
use crate::request::{Platform, Resolve};

/// Source configuration for the build pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sources {
    pub registry: String,
    pub installer: String,
}

/// Resolves a profile and request into versioned OCI references.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Resolver {
    registry: String,
    installer: String,
}

impl Resolver {
    /// Creates a source resolver from imager source configuration.
    #[must_use]
    fn new(sources: &Sources) -> Self {
        Self {
            registry: sources.registry.clone(),
            installer: sources.installer.clone(),
        }
    }

    /// Resolves a build request and profile into concrete source references.
    ///
    /// # Errors
    ///
    /// Returns an error when the profile references an unknown source input.
    fn resolve(&self, request: &Resolve, profile: &Profile) -> Result<ResolvedBuildProfile> {
        let mut extensions = profile
            .customization
            .extensions
            .iter()
            .map(|name| self.resolve_one_extension(name, &request.version))
            .collect::<Result<Vec<_>>>()?;
        extensions.sort_unstable_by(|left, right| left.name().cmp(right.name()));

        let overlay = profile.overlay.as_ref().map(|overlay_spec| {
            ResolvedOverlay::new(
                overlay_spec.name.clone(),
                overlay_spec.image.clone(),
                self.versioned_ref(&overlay_spec.image, &request.version),
            )
        });

        Ok(ResolvedBuildProfile::new(
            request.platform,
            request.version.clone(),
            request.arch,
            extensions,
            overlay,
            self.versioned_ref(&self.installer, &request.version),
        ))
    }

    /// Builds a versioned OCI reference for a logical repository.
    fn versioned_ref(&self, repository: &str, version: &str) -> String {
        format!("{}/{repository}:{version}", self.registry)
    }

    /// Resolves a single extension name.
    fn resolve_one_extension(&self, name: &str, version: &str) -> Result<ResolvedExtension> {
        let normalized = resolve_extension_name(name);
        if !is_official_extension(normalized) {
            return Err(ImagerError::SourceResolution(format!(
                "unknown official extension: {name}"
            )));
        }
        Ok(ResolvedExtension::new(
            normalized.to_owned(),
            self.versioned_ref(normalized, version),
        ))
    }
}

/// Resolved extension source.
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

/// Resolves a profile and request into versioned OCI references.
///
/// # Errors
///
/// Returns an error when the profile references an unknown source input.
pub fn resolve(
    request: &Resolve,
    profile: &Profile,
    sources: &Sources,
) -> Result<ResolvedBuildProfile> {
    let resolver = Resolver::new(sources);
    resolver.resolve(request, profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Profile;
    use crate::request::{Platform, Resolve};

    fn sources() -> Sources {
        Sources {
            registry: "ghcr.io".into(),
            installer: "muak-os/installer".into(),
        }
    }

    #[test]
    fn resolve_build_profile_uses_versioned_installer() {
        // ARRANGE
        let request = Resolve {
            version: "v1.0.0-beta".into(),
            platform: Platform::Metal,
            arch: Arch::Amd64,
        };
        let profile = Profile::from_toml(b"[customization]\nextensions = []").expect("parse");

        // ACT
        let bp = resolve(&request, &profile, &sources()).expect("resolve");

        // ASSERT
        assert_eq!(bp.installer(), "ghcr.io/muak-os/installer:v1.0.0-beta");
        assert_eq!(bp.version(), "v1.0.0-beta");
        assert_eq!(bp.arch(), Arch::Amd64);
        assert_eq!(bp.platform(), Platform::Metal);
    }

    #[test]
    fn resolve_build_profile_sorts_extensions() {
        // ARRANGE
        let request = Resolve {
            version: "v1.0.0-beta".into(),
            platform: Platform::Metal,
            arch: Arch::Amd64,
        };
        let profile =
            Profile::from_toml(b"[customization]\nextensions = [\"muak-os/qemu\"]").expect("parse");

        // ACT
        let bp = resolve(&request, &profile, &sources()).expect("resolve");

        // ASSERT
        assert_eq!(bp.extensions().len(), 1);
        assert_eq!(
            bp.extensions().first().expect("first ext").name(),
            "muak-os/qemu"
        );
    }

    #[test]
    fn resolve_rejects_unknown_extension() {
        // ARRANGE
        let request = Resolve {
            version: "v1.0.0-beta".into(),
            platform: Platform::Metal,
            arch: Arch::Amd64,
        };
        let profile =
            Profile::from_toml(b"[customization]\nextensions = [\"custom/thing\"]").expect("parse");

        // ACT
        let result = resolve(&request, &profile, &sources());

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|e| e.to_string().contains("unknown official extension"))
        );
    }

    #[test]
    fn resolve_with_overlay() {
        // ARRANGE
        let request = Resolve {
            version: "v1.0.0-beta".into(),
            platform: Platform::Metal,
            arch: Arch::Amd64,
        };
        let profile = Profile::from_toml(
            b"[overlay]\nname = \"rpi\"\nimage = \"muak-os/sbc\"\n[customization]\nextensions = []",
        )
        .expect("parse");

        // ACT
        let bp = resolve(&request, &profile, &sources()).expect("resolve");

        // ASSERT
        assert!(bp.overlay().is_some());
        let ov = bp.overlay().expect("overlay");
        assert_eq!(ov.name(), "rpi");
        assert_eq!(ov.image(), "muak-os/sbc");
        assert_eq!(ov.source_ref(), "ghcr.io/muak-os/sbc:v1.0.0-beta");
    }

    #[test]
    fn resolve_aliases_extension_name() {
        // ARRANGE
        let request = Resolve {
            version: "v1.0.0".into(),
            platform: Platform::Metal,
            arch: Arch::Amd64,
        };
        let profile =
            Profile::from_toml(b"[customization]\nextensions = [\"qemu\"]").expect("parse");

        // ACT
        let bp = resolve(&request, &profile, &sources()).expect("resolve");

        // ASSERT
        assert_eq!(bp.extensions().len(), 1);
        assert_eq!(bp.extensions().first().expect("ext").name(), "muak-os/qemu");
    }

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
        assert_eq!(bp.installer(), "ghcr.io/muak-os/installer:v1.0.0-beta");
    }
}
