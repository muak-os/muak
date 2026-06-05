//! Source resolution logic for the build pipeline.

use crate::catalog::{is_official_extension, resolve_extension_name};
use crate::error::{ImagerError, Result};
use crate::profile::Profile;
use crate::request::Request;
use crate::source::model::{ResolvedBuildProfile, ResolvedExtension, ResolvedOverlay};

/// Source configuration for the build pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sources {
    pub registry: String,
    pub installer: String,
}

/// Resolves a profile and request into versioned OCI references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolver {
    registry: String,
    installer: String,
}

impl Resolver {
    /// Creates a source resolver from imager source configuration.
    #[must_use]
    pub fn new(sources: &Sources) -> Self {
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
    pub fn resolve(&self, request: &Request, profile: &Profile) -> Result<ResolvedBuildProfile> {
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

#[cfg(test)]
mod tests {
    use koci::arch::Arch;

    use super::*;
    use crate::profile::Profile;
    use crate::request::{Artifact, Platform, Request};

    fn sources() -> Sources {
        Sources {
            registry: "ghcr.io".into(),
            installer: "muak-os/installer".into(),
        }
    }

    fn resolver() -> Resolver {
        Resolver::new(&sources())
    }

    #[test]
    fn resolve_build_profile_uses_versioned_installer() {
        // ARRANGE
        let request = Request {
            profile_id: "abc".into(),
            version: "v1.0.0-beta".into(),
            artifact: Artifact::Uki,
            platform: Platform::Metal,
            arch: Arch::Amd64,
        };
        let profile = Profile::from_toml(b"[customization]\nextensions = []").expect("parse");

        // ACT
        let bp = resolver().resolve(&request, &profile).expect("resolve");

        // ASSERT
        assert_eq!(bp.installer(), "ghcr.io/muak-os/installer:v1.0.0-beta");
        assert_eq!(bp.version(), "v1.0.0-beta");
        assert_eq!(bp.arch(), Arch::Amd64);
        assert_eq!(bp.platform(), Platform::Metal);
    }

    #[test]
    fn resolve_build_profile_sorts_extensions() {
        // ARRANGE
        let request = Request {
            profile_id: "abc".into(),
            version: "v1.0.0-beta".into(),
            artifact: Artifact::Uki,
            platform: Platform::Metal,
            arch: Arch::Amd64,
        };
        let profile =
            Profile::from_toml(b"[customization]\nextensions = [\"muak-os/qemu\"]").expect("parse");

        // ACT
        let bp = resolver().resolve(&request, &profile).expect("resolve");

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
        let request = Request {
            profile_id: "abc".into(),
            version: "v1.0.0-beta".into(),
            artifact: Artifact::Uki,
            platform: Platform::Metal,
            arch: Arch::Amd64,
        };
        let profile =
            Profile::from_toml(b"[customization]\nextensions = [\"custom/thing\"]").expect("parse");

        // ACT
        let result = resolver().resolve(&request, &profile);

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
        let request = Request {
            profile_id: "abc".into(),
            version: "v1.0.0-beta".into(),
            artifact: Artifact::Iso,
            platform: Platform::Metal,
            arch: Arch::Amd64,
        };
        let profile = Profile::from_toml(
            b"[overlay]\nname = \"rpi\"\nimage = \"muak-os/sbc\"\n[customization]\nextensions = []",
        )
        .expect("parse");

        // ACT
        let bp = resolver().resolve(&request, &profile).expect("resolve");

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
        let request = Request {
            profile_id: "abc".into(),
            version: "v1.0.0".into(),
            artifact: Artifact::Uki,
            platform: Platform::Metal,
            arch: Arch::Amd64,
        };
        let profile =
            Profile::from_toml(b"[customization]\nextensions = [\"qemu\"]").expect("parse");

        // ACT
        let bp = resolver().resolve(&request, &profile).expect("resolve");

        // ASSERT
        assert_eq!(bp.extensions().len(), 1);
        assert_eq!(bp.extensions().first().expect("ext").name(), "muak-os/qemu");
    }
}
