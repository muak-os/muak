//! Resolver: derives a complete resolution from a request and profile.

mod catalog;
pub mod discover;
mod inputs;

use catalog::{fetch, fetch_tag, pinned_reference};
use inputs::{extensions, overlay};
use kata::schema::kinds::Kind;
use kata::schema::parse::validate_release;
use kata::schema::view::{self, Role};
use koci::arch;

use crate::domain::catalog::Catalog;
use crate::domain::identity::{ResolutionId, ResolvedInput};
use crate::domain::input::Kernel;
use crate::domain::profile::Profile;
use crate::domain::resolution::{Resolution, ResolvedBuild};
use crate::error::{Result, WizardError};
use crate::request::Request;

/// Resolution policy identifier, part of the resolution identity.
const RESOLUTION_POLICY: &str = "muak/default";

/// Resolves a request and profile into a complete resolution.
///
/// # Errors
///
/// Returns an error when the release version is invalid or a catalog cannot be
/// fetched or does not contain a selected entry.
pub fn plan(request: &Request, profile: &Profile) -> Result<Resolution> {
    let registry = request.registry();
    let arch = request.target_arch().unwrap_or_else(arch::host);
    let release = request.version();
    valid_release(release)?;
    let profile_id = profile.profile_id()?;

    let core = fetch_tag(Kind::Core, release, registry)?;
    if core.document.release() != release {
        return Err(WizardError::SourceResolution(format!(
            "catalog release '{}' does not match '{release}'",
            core.document.release()
        )));
    }
    let kernel_entry = core.document.kernel(profile.kernel().source())?;
    let stub_entry = core.document.stub()?;
    let installer_entry = core.document.installer()?;

    let mut resolved_inputs: Vec<ResolvedInput> = vec![
        view::sourced(Role::Kernel, kernel_entry).into(),
        view::sourced(Role::Stub, stub_entry).into(),
        view::sourced(Role::Installer, installer_entry).into(),
    ];

    let kernel = Kernel::new(
        profile.kernel().source().to_owned(),
        pinned_reference(registry, &kernel_entry.repository, &kernel_entry.digest),
    );
    let stub_reference = pinned_reference(registry, &stub_entry.repository, &stub_entry.digest);
    let installer_reference = pinned_reference(
        registry,
        &installer_entry.repository,
        &installer_entry.digest,
    );

    let selected_extensions = if profile.customization().extensions().is_empty() {
        Vec::new()
    } else {
        let document = fetch(Kind::Extensions, release, registry)?;
        extensions(&document, profile, registry, &mut resolved_inputs)?
    };
    let selected_overlay = match profile.overlay() {
        Some(spec) => {
            let document = fetch(Kind::Overlays, release, registry)?;
            overlay(&document, spec, arch, registry, &mut resolved_inputs)?
        }
        None => None,
    };

    let build = ResolvedBuild::new(release.to_owned(), arch, kernel).with_sources(
        stub_reference,
        installer_reference,
        selected_overlay,
        selected_extensions,
    );
    let resolution_id = ResolutionId::compute(
        &profile_id,
        &resolved_inputs,
        arch.as_str(),
        RESOLUTION_POLICY,
    );
    let catalog = Catalog::new(format!("{registry}/{}", Kind::Core.dir()), core.digest);

    Ok(Resolution::new(
        profile_id,
        resolution_id,
        resolved_inputs,
        catalog,
        build,
    ))
}

/// Resolves a catalog tag to the concrete release line it pins.
///
/// # Errors
///
/// Returns an error when the catalog image cannot be fetched or parsed.
pub fn release_of(registry: &str, tag: &str) -> Result<String> {
    Ok(fetch_tag(Kind::Core, tag, registry)?
        .document
        .release()
        .to_owned())
}

/// Checks that `registry` answers requests for the core catalog at `tag`.
///
/// # Errors
///
/// Returns an error when the registry cannot be contacted or authenticated.
pub fn reachable(registry: &str, tag: &str) -> Result<()> {
    let reference = format!("{registry}/{}:{tag}", Kind::Core.dir());
    koci::registry::manifest_exists(&reference).map_err(|error| {
        WizardError::SourceResolution(format!("check catalog image {reference}: {error}"))
    })?;

    Ok(())
}

fn valid_release(version: &str) -> Result<()> {
    validate_release(version).map_err(WizardError::from)
}

#[cfg(test)]
mod tests {
    use kata::schema::documents::Document;
    use kata::schema::parse::from_toml;

    use super::*;

    const RELEASE: &str = "v1.2.3";

    const CORE: &str = r#"api_version = "muak.dev/catalog/core/v1"
release = "v1.2.3"

[[kernels]]
source = "muak-os/linux"
repository = "linux"
tag = "v6.12.4-muak1"
digest = "sha256:1111"

[stub]
source = "muak-os/stub"
repository = "stub"
tag = "v0.3.1"
digest = "sha256:2222"

[installer]
source = "muak-os/installer"
repository = "installer"
tag = "v1.2.3"
digest = "sha256:3333"
"#;

    fn document(kind: Kind, body: &str) -> Document {
        from_toml(kind, body.as_bytes(), RELEASE).expect("parse fixture document")
    }

    #[test]
    fn core_lookups_resolve_entries_by_source() {
        // ARRANGE
        let core = document(Kind::Core, CORE);

        // ACT / ASSERT
        assert_eq!(
            core.kernel("muak-os/linux").expect("kernel").digest,
            "sha256:1111"
        );
        assert_eq!(core.stub().expect("stub").digest, "sha256:2222");
        assert_eq!(core.installer().expect("installer").digest, "sha256:3333");
        core.kernel("other/kernel").unwrap_err();
    }

    #[test]
    fn core_lookups_reject_missing_frozen_entries() {
        // ARRANGE
        let core_without_stub = r#"api_version = "muak.dev/catalog/core/v1"
release = "v1.2.3"

[[kernels]]
source = "muak-os/linux"
repository = "linux"
tag = "v6.12.4-muak1"
digest = "sha256:1111"
"#;
        let core = document(Kind::Core, core_without_stub);

        // ACT / ASSERT
        let error = core.stub().expect_err("missing stub should fail");
        assert!(error.to_string().contains("missing the stub entry"));
    }

    #[test]
    fn valid_release_accepts_tags_and_rejects_reference_delimiters() {
        // ARRANGE / ACT / ASSERT
        valid_release("v1.2.3").expect("tag release accepted");
        valid_release("latest").expect("latest accepted");
        for version in ["", "v1:latest", "v1/latest", "v1 latest"] {
            let error = valid_release(version).expect_err("should fail");
            assert!(error.to_string().contains("invalid release version"));
        }
    }
}
