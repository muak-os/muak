//! OCI reference resolution engine.

use koci::arch::Arch;

use crate::config::Sources;
use crate::error::{Result, WizardError};
use crate::profile::Profile;
use crate::request::Platform;
use crate::resolve::BuildPlan;
use crate::source::extension::Extension;
use crate::source::overlay::Overlay;

const OFFICIAL_EXTENSION_REPOSITORIES: &[&str] = &["muak-os/qemu"];

/// Resolves a profile and request into versioned OCI references.
///
/// # Errors
///
/// Returns an error when the profile references an unknown source input.
pub(super) fn resolve(
    version: &str,
    platform: Platform,
    arch: Arch,
    profile: &Profile,
    sources: &Sources,
) -> Result<BuildPlan> {
    let mut extensions = profile
        .customization()
        .extensions()
        .iter()
        .map(|name| resolve_one_extension(name, version, &sources.registry))
        .collect::<Result<Vec<_>>>()?;
    extensions.sort_unstable_by(|left, right| left.name().cmp(right.name()));

    let overlay = profile.overlay().map(|overlay_spec| {
        Overlay::new(
            overlay_spec.name().to_owned(),
            overlay_spec.image().to_owned(),
            versioned_ref(overlay_spec.image(), version, &sources.registry),
            arch,
        )
    });

    Ok(BuildPlan::new(
        platform,
        version.to_owned(),
        arch,
        extensions,
        overlay,
        versioned_ref(&sources.installer, version, &sources.registry),
    ))
}

/// Normalizes legacy extension names to canonical logical names.
fn resolve_extension_name(name: &str) -> &str {
    match name {
        "qemu" => "muak-os/qemu",
        other => other,
    }
}

/// Returns true when a logical extension belongs to the checked-in official inventory.
fn is_official_extension(name: &str) -> bool {
    OFFICIAL_EXTENSION_REPOSITORIES.binary_search(&name).is_ok()
}

fn versioned_ref(repository: &str, version: &str, registry: &str) -> String {
    format!("{registry}/{repository}:{version}")
}

fn resolve_one_extension(name: &str, version: &str, registry: &str) -> Result<Extension> {
    let normalized = resolve_extension_name(name);
    if !is_official_extension(normalized) {
        return Err(WizardError::SourceResolution(format!(
            "unknown official extension: {name}"
        )));
    }
    Ok(Extension::new(
        normalized.to_owned(),
        versioned_ref(normalized, version, registry),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Profile;
    use crate::request::Platform;

    fn sources() -> Sources {
        Sources {
            registry: "ghcr.io".into(),
            installer: "muak-os/installer".into(),
        }
    }

    #[test]
    fn uses_versioned_installer() {
        // ARRANGE
        let request_version = "v1.0.0-beta";
        let profile = Profile::from_toml(b"[customization]\nextensions = []").expect("parse");

        // ACT
        let bp = resolve(
            request_version,
            Platform::Metal,
            Arch::Amd64,
            &profile,
            &sources(),
        )
        .expect("resolve");

        // ASSERT
        assert_eq!(bp.installer(), "ghcr.io/muak-os/installer:v1.0.0-beta");
        assert_eq!(bp.version(), "v1.0.0-beta");
        assert_eq!(bp.arch(), Arch::Amd64);
        assert_eq!(bp.platform(), Platform::Metal);
    }

    #[test]
    fn sorts_extensions() {
        // ARRANGE
        let request_version = "v1.0.0-beta";
        let profile =
            Profile::from_toml(b"[customization]\nextensions = [\"muak-os/qemu\"]").expect("parse");

        // ACT
        let bp = resolve(
            request_version,
            Platform::Metal,
            Arch::Amd64,
            &profile,
            &sources(),
        )
        .expect("resolve");

        // ASSERT
        assert_eq!(bp.extensions().len(), 1);
        assert_eq!(
            bp.extensions().first().expect("first ext").name(),
            "muak-os/qemu"
        );
    }

    #[test]
    fn rejects_unknown_extension() {
        // ARRANGE
        let request_version = "v1.0.0-beta";
        let profile =
            Profile::from_toml(b"[customization]\nextensions = [\"custom/thing\"]").expect("parse");

        // ACT
        let result = resolve(
            request_version,
            Platform::Metal,
            Arch::Amd64,
            &profile,
            &sources(),
        );

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|e| e.to_string().contains("unknown official extension"))
        );
    }

    #[test]
    fn resolves_overlay() {
        // ARRANGE
        let request_version = "v1.0.0-beta";
        let profile = Profile::from_toml(
            b"[overlay]\nname = \"rpi\"\nimage = \"muak-os/sbc\"\n[customization]\nextensions = []",
        )
        .expect("parse");

        // ACT
        let bp = resolve(
            request_version,
            Platform::Metal,
            Arch::Amd64,
            &profile,
            &sources(),
        )
        .expect("resolve");

        // ASSERT
        assert!(bp.overlay().is_some());
        let ov = bp.overlay().expect("overlay");
        assert_eq!(ov.name(), "rpi");
        assert_eq!(ov.image(), "muak-os/sbc");
        assert_eq!(ov.source_ref(), "ghcr.io/muak-os/sbc:v1.0.0-beta");
    }

    #[test]
    fn aliases_extension_name() {
        // ARRANGE
        let request_version = "v1.0.0";
        let profile =
            Profile::from_toml(b"[customization]\nextensions = [\"qemu\"]").expect("parse");

        // ACT
        let bp = resolve(
            request_version,
            Platform::Metal,
            Arch::Amd64,
            &profile,
            &sources(),
        )
        .expect("resolve");

        // ASSERT
        assert_eq!(bp.extensions().len(), 1);
        assert_eq!(bp.extensions().first().expect("ext").name(), "muak-os/qemu");
    }

    #[test]
    fn different_overlay_names_produce_different_resolved_overlays() {
        // ARRANGE
        let request_version = "v1.0.0";
        let profile_a = Profile::from_toml(
            b"[overlay]\nname = \"rpi-4\"\nimage = \"muak-os/sbc\"\n[customization]\nextensions = []",
        )
        .expect("parse");
        let profile_b = Profile::from_toml(
            b"[overlay]\nname = \"rpi-5\"\nimage = \"muak-os/sbc\"\n[customization]\nextensions = []",
        )
        .expect("parse");

        // ACT
        let bp_a = resolve(
            request_version,
            Platform::Metal,
            Arch::Amd64,
            &profile_a,
            &sources(),
        )
        .expect("resolve");
        let bp_b = resolve(
            request_version,
            Platform::Metal,
            Arch::Amd64,
            &profile_b,
            &sources(),
        )
        .expect("resolve");

        // ASSERT
        assert_eq!(bp_a.overlay().expect("overlay a").name(), "rpi-4");
        assert_eq!(bp_b.overlay().expect("overlay b").name(), "rpi-5");
        assert_eq!(
            bp_a.overlay().expect("overlay a").image(),
            bp_b.overlay().expect("overlay b").image()
        );
        assert_ne!(bp_a.overlay().expect("overlay a").source_ref(), "");
    }

    #[test]
    fn resolves_legacy_extension_names() {
        assert_eq!(resolve_extension_name("qemu"), "muak-os/qemu");
        assert_eq!(resolve_extension_name("custom"), "custom");
        assert_eq!(resolve_extension_name("muak-os/qemu"), "muak-os/qemu");
    }

    #[test]
    fn identifies_official_extensions() {
        assert!(is_official_extension("muak-os/qemu"));
        assert!(!is_official_extension("custom/thing"));
    }
}
