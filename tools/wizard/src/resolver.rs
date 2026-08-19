//! Resolver: derives phase 3 (resolution) from the domain model.

use koci::arch::{self, Arch};

use crate::config;
use crate::domain::identity::ResolutionId;
use crate::domain::profile::{Profile, normalize_extension_name};
use crate::domain::release::{Manifest, manifest_for_version};
use crate::domain::resolution::{Extension, Kernel, Overlay, Resolution, ResolvedBuild};
use crate::error::{Result, WizardError};
use crate::request::Request;

/// Resolution policy identifier, part of the resolution identity.
const RESOLUTION_POLICY: &str = "muak/default";

/// Resolves a request and profile into a complete resolution.
///
/// # Errors
///
/// Returns an error when the request version is invalid, the profile references
/// an unknown source input, or the global configuration has not been set.
pub(crate) fn plan(request: &Request, profile: &Profile) -> Result<Resolution> {
    let config = config::config()?;
    let host = arch::host();
    let arch = request.target_arch().unwrap_or(host);
    let manifest = manifest_for_version(request.version())?;

    let profile_id = profile.profile_id()?;
    let release_id = manifest.id()?;

    let registry = &config.registry;
    let installer = manifest.installer().reference(registry);
    let kernel = match_kernel(profile, &manifest, registry)?;
    let extensions = match_extensions(profile, &manifest, registry)?;
    let overlay = match_overlay(profile, &manifest, registry, arch)?;

    let build = ResolvedBuild::new(
        request.platform(),
        manifest.version().to_owned(),
        arch,
        extensions,
        overlay,
        kernel,
        installer,
    );
    let resolution_id = ResolutionId::compute(
        &profile_id,
        &release_id,
        arch.as_str(),
        request.platform().as_str(),
        RESOLUTION_POLICY,
    );

    Ok(Resolution::new(
        profile_id,
        release_id,
        resolution_id,
        build,
    ))
}

/// Matches the profile kernel identity against the manifest.
///
/// # Errors
///
/// Returns an error when the profile kernel source is missing from the manifest.
fn match_kernel(profile: &Profile, manifest: &Manifest, registry: &str) -> Result<Kernel> {
    let source = profile.kernel().source();
    if source != manifest.kernel().source() {
        return Err(WizardError::SourceResolution(format!(
            "manifest '{}' does not contain kernel source '{source}'",
            manifest.name()
        )));
    }

    Ok(Kernel::new(
        source.to_owned(),
        manifest.kernel().reference(registry),
    ))
}

/// Matches the normalized profile extensions against manifest entries.
///
/// # Errors
///
/// Returns an error when any extension is missing from the manifest.
fn match_extensions(
    profile: &Profile,
    manifest: &Manifest,
    registry: &str,
) -> Result<Vec<Extension>> {
    profile
        .customization()
        .extensions()
        .iter()
        .map(|name| normalize_extension_name(name))
        .map(|source| {
            let entry = manifest
                .extensions()
                .iter()
                .find(|entry| entry.source() == source)
                .ok_or_else(|| {
                    WizardError::SourceResolution(format!(
                        "manifest '{}' does not contain extension '{source}'",
                        manifest.name()
                    ))
                })?;
            Ok(Extension::new(source.to_owned(), entry.reference(registry)))
        })
        .collect::<Result<Vec<_>>>()
}

/// Matches the profile overlay identity against the manifest.
///
/// # Errors
///
/// Returns an error when the profile overlay source or name is missing from
/// the manifest.
fn match_overlay(
    profile: &Profile,
    manifest: &Manifest,
    registry: &str,
    arch: Arch,
) -> Result<Option<Overlay>> {
    let Some(spec) = profile.overlay() else {
        return Ok(None);
    };

    let entry = manifest
        .overlays()
        .iter()
        .find(|entry| entry.source() == spec.source() && entry.name() == spec.name())
        .ok_or_else(|| {
            WizardError::SourceResolution(format!(
                "manifest '{}' does not contain overlay '{}/{}'",
                manifest.name(),
                spec.name(),
                spec.source()
            ))
        })?;

    Ok(Some(Overlay::new(
        spec.name().to_owned(),
        spec.source().to_owned(),
        entry.reference(registry),
        arch,
    )))
}

#[cfg(test)]
mod tests {
    use std::sync::Once;

    use super::*;
    use crate::config;
    use crate::domain::profile::{CustomizationSpec, KernelSpec, OverlaySpec, Profile};
    use crate::request::Platform;

    static CONFIGURE: Once = Once::new();

    fn configure() {
        CONFIGURE.call_once(|| {
            config::configure(config::Config {
                cache_dir: None,
                registry: "ghcr.io/muak-os".to_owned(),
            })
            .expect("configure");
        });
    }

    /// Resolves via the public entry point against the single configure registry.
    fn resolve(profile: &Profile, version: &str, arch: Arch) -> Result<Resolution> {
        configure();
        let request = Request::new(version, Platform::Metal).arch(arch);

        plan(&request, profile)
    }

    fn profile(overlay: Option<OverlaySpec>, extensions: &[&str]) -> Profile {
        let customization =
            CustomizationSpec::new(extensions.iter().map(|name| (*name).to_owned()).collect())
                .expect("customization");
        let kernel = KernelSpec::new("muak-os/kernel".into()).expect("kernel");

        Profile::new(overlay, customization, kernel)
    }

    fn base_profile() -> Profile {
        profile(None, &[])
    }

    fn overlay_profile() -> Profile {
        let overlay = OverlaySpec::new("rpi_generic".into(), "muak-os/sbc-raspberrypi".into())
            .expect("overlay");

        profile(Some(overlay), &[])
    }

    #[test]
    fn resolves_references_from_manifest() {
        // ARRANGE / ACT
        let resolution = resolve(&base_profile(), "latest", Arch::Amd64).expect("resolve");

        // ASSERT
        let build = resolution.build();
        assert_eq!(build.installer(), "ghcr.io/muak-os/installer:latest");
        assert_eq!(build.kernel().source(), "ghcr.io/muak-os/kernel:latest");
        assert_eq!(build.kernel().image(), "muak-os/kernel");
        assert_eq!(build.version(), "latest");
        assert_eq!(build.arch(), Arch::Amd64);
        assert_eq!(build.platform(), Platform::Metal);
    }

    #[test]
    fn resolution_ids_are_computed() {
        // ARRANGE / ACT
        let resolution = resolve(&base_profile(), "latest", Arch::Amd64).expect("resolve");

        // ASSERT
        assert_eq!(resolution.profile_id().to_string().len(), 64);
        assert_eq!(resolution.release_id().to_string().len(), 64);
        assert_eq!(resolution.resolution_id().to_string().len(), 64);
    }

    #[test]
    fn arch_change_affects_resolution_id_only() {
        // ARRANGE / ACT
        let amd64 = resolve(&base_profile(), "latest", Arch::Amd64).expect("resolve amd64");
        let arm64 = resolve(&base_profile(), "latest", Arch::Arm64).expect("resolve arm64");

        // ASSERT
        assert_eq!(amd64.profile_id(), arm64.profile_id());
        assert_eq!(amd64.release_id(), arm64.release_id());
        assert_ne!(amd64.resolution_id(), arm64.resolution_id());
    }

    #[test]
    fn resolves_extensions_from_manifest() {
        // ARRANGE / ACT
        let resolution =
            resolve(&profile(None, &["muak-os/qemu"]), "latest", Arch::Amd64).expect("resolve");

        // ASSERT
        assert_eq!(resolution.build().extensions().len(), 1);
        let ext = resolution.build().extensions().first().expect("ext");
        assert_eq!(ext.name(), "muak-os/qemu");
        assert_eq!(ext.source(), "ghcr.io/muak-os/pkgs/qemu:latest");
    }

    #[test]
    fn aliases_extension_name() {
        // ARRANGE / ACT
        let resolution =
            resolve(&profile(None, &["qemu"]), "latest", Arch::Amd64).expect("resolve");

        // ASSERT
        assert_eq!(resolution.build().extensions().len(), 1);
        assert_eq!(
            resolution.build().extensions().first().expect("ext").name(),
            "muak-os/qemu"
        );
    }

    #[test]
    fn rejects_unknown_extension() {
        // ARRANGE / ACT
        let result = resolve(&profile(None, &["custom/thing"]), "latest", Arch::Amd64);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|e| e.to_string().contains("does not contain extension"))
        );
    }

    #[test]
    fn resolves_overlay_from_manifest() {
        // ARRANGE / ACT
        let resolution = resolve(&overlay_profile(), "latest", Arch::Amd64).expect("resolve");

        // ASSERT
        let overlay = resolution.build().overlay().expect("overlay");
        assert_eq!(overlay.name(), "rpi_generic");
        assert_eq!(overlay.image(), "muak-os/sbc-raspberrypi");
        assert_eq!(
            overlay.source_ref(),
            "ghcr.io/muak-os/pkgs/sbc-raspberrypi:latest"
        );
    }

    #[test]
    fn rejects_mismatched_overlay_source() {
        // ARRANGE
        let overlay = OverlaySpec::new("rpi_generic".into(), "other/sbc".into()).expect("overlay");
        let profile = profile(Some(overlay), &[]);

        // ACT
        let result = resolve(&profile, "latest", Arch::Amd64);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|e| e.to_string().contains("does not contain overlay"))
        );
    }

    #[test]
    fn arbitrary_version_resolves() {
        // ARRANGE / ACT
        let resolution = resolve(&base_profile(), "v2.0.0", Arch::Amd64).expect("resolve");

        // ASSERT
        assert_eq!(resolution.build().version(), "v2.0.0");
        assert_eq!(
            resolution.build().installer(),
            "ghcr.io/muak-os/installer:v2.0.0"
        );
        assert_eq!(
            resolution.build().kernel().source(),
            "ghcr.io/muak-os/kernel:v2.0.0"
        );
    }

    #[test]
    fn rejects_invalid_version() {
        // ARRANGE / ACT
        for (label, version) in [
            ("empty", ""),
            ("colon", "v1:latest"),
            ("slash", "v1/latest"),
            ("whitespace", "v1 latest"),
        ] {
            let result = resolve(&base_profile(), version, Arch::Amd64);

            // ASSERT
            assert!(
                result
                    .as_ref()
                    .is_err_and(|e| e.to_string().contains("invalid release version")),
                "{label}: expected an invalid version error"
            );
        }
    }
}
