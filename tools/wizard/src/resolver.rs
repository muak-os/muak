//! Resolver: derives phase 3 (resolution) from the domain model.

use kata::schema::documents::{DOCUMENT_PATH, Document};
use kata::schema::entries::NamedEntry;
use kata::schema::kinds::Kind;
use kata::schema::parse::{from_toml, validate_release};
use kata::schema::view::{self, Role};
use koci::arch::{self, Arch};

use crate::config;
use crate::domain::identity::{ResolutionId, ResolvedInput};
use crate::domain::profile::{OverlaySpec, Profile, normalize_extension_name};
use crate::domain::resolution::{Extension, Kernel, Overlay, Resolution, ResolvedBuild};
use crate::error::{Result, WizardError};
use crate::request::Request;

/// Resolution policy identifier, part of the resolution identity.
const RESOLUTION_POLICY: &str = "muak/default";

/// Resolves a request and profile into a complete resolution.
///
/// # Errors
///
/// Returns an error when the release version is invalid, a catalog cannot be fetched
/// or does not contain a selected entry, or the global configuration has not been set.
pub fn plan(request: &Request, profile: &Profile) -> Result<Resolution> {
    let config = config::config()?;
    let arch = request.target_arch().unwrap_or_else(arch::host);
    valid_release(request.version())?;
    let release = request.version();
    let profile_id = profile.profile_id()?;

    let core = fetch(Kind::Core, release, &config.registry)?;
    let kernel_entry = core.kernel(profile.kernel().source())?;
    let stub_entry = core.stub()?;
    let installer_entry = core.installer()?;

    let mut inputs: Vec<ResolvedInput> = vec![
        view::sourced(Role::Kernel, kernel_entry).into(),
        view::sourced(Role::Stub, stub_entry).into(),
        view::sourced(Role::Installer, installer_entry).into(),
    ];

    let kernel = Kernel::new(
        profile.kernel().source().to_owned(),
        pinned_reference(
            &config.registry,
            &kernel_entry.repository,
            &kernel_entry.digest,
        ),
    );
    let stub_reference =
        pinned_reference(&config.registry, &stub_entry.repository, &stub_entry.digest);
    let installer_reference = pinned_reference(
        &config.registry,
        &installer_entry.repository,
        &installer_entry.digest,
    );

    let extensions = if profile.customization().extensions().is_empty() {
        Vec::new()
    } else {
        let document = fetch(Kind::Extensions, release, &config.registry)?;
        match_extensions(&document, profile, &config.registry, &mut inputs)?
    };
    let overlay = match profile.overlay() {
        Some(spec) => {
            let document = fetch(Kind::Overlays, release, &config.registry)?;
            match_overlay(&document, spec, arch, &config.registry, &mut inputs)?
        }
        None => None,
    };

    let build = ResolvedBuild::new(release.to_owned(), arch, kernel).with_sources(
        stub_reference,
        installer_reference,
        overlay,
        extensions,
    );
    let resolution_id =
        ResolutionId::compute(&profile_id, &inputs, arch.as_str(), RESOLUTION_POLICY);

    Ok(Resolution::new(profile_id, resolution_id, build))
}

fn match_extensions(
    document: &Document,
    profile: &Profile,
    registry: &str,
    inputs: &mut Vec<ResolvedInput>,
) -> Result<Vec<Extension>> {
    profile
        .customization()
        .extensions()
        .iter()
        .map(|name| {
            let name = normalize_extension_name(name);
            let entry = document.named(name).ok_or_else(|| {
                WizardError::SourceResolution(format!(
                    "catalog does not contain extension '{name}'"
                ))
            })?;
            inputs.push(view::named(Role::Extension, entry).into());

            Ok(Extension::new(
                name.to_owned(),
                pinned_reference(registry, &entry.repository, &entry.digest),
            ))
        })
        .collect()
}

fn match_overlay(
    document: &Document,
    spec: &OverlaySpec,
    arch: Arch,
    registry: &str,
    inputs: &mut Vec<ResolvedInput>,
) -> Result<Option<Overlay>> {
    let entry: &NamedEntry = document.named(spec.name()).ok_or_else(|| {
        WizardError::SourceResolution(format!(
            "catalog does not contain overlay '{}'",
            spec.name()
        ))
    })?;
    inputs.push(view::named(Role::Overlay, entry).into());

    Ok(Some(Overlay::new(
        spec.name().to_owned(),
        entry.source.clone(),
        pinned_reference(registry, &entry.repository, &entry.digest),
        arch,
    )))
}

fn fetch(kind: Kind, release: &str, registry: &str) -> Result<Document> {
    let reference = format!("{registry}/{}:{release}", kind.repository());
    let mut document: Option<Vec<u8>> = None;
    koci::pull::files(&reference, &Arch::Amd64, None, |entry| {
        if entry.path == DOCUMENT_PATH {
            let mut buffer = Vec::new();
            entry.reader.read_to_end(&mut buffer)?;
            document = Some(buffer);
        }

        Ok(())
    })
    .map_err(|error| {
        WizardError::SourceResolution(format!("fetch catalog image {reference}: {error}"))
    })?;
    let bytes = document.ok_or_else(|| {
        WizardError::SourceResolution(format!(
            "catalog image {reference} does not contain {DOCUMENT_PATH}",
        ))
    })?;

    Ok(from_toml(kind, &bytes, release)?)
}

fn pinned_reference(registry: &str, repository: &str, digest: &str) -> String {
    format!("{registry}/{repository}@{digest}")
}

fn valid_release(version: &str) -> Result<()> {
    validate_release(version).map_err(WizardError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::profile::{CustomizationSpec, KernelSpec, Profile};

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

    const EXTENSIONS: &str = r#"api_version = "muak.dev/catalog/extensions/v1"
release = "v1.2.3"

[[extensions]]
name = "muak-os/qemu"
source = "muak-os/extensions"
repository = "extensions/qemu"
tag = "v0.2.1"
digest = "sha256:4444"
"#;

    const OVERLAYS: &str = r#"api_version = "muak.dev/catalog/overlays/v1"
release = "v1.2.3"

[[overlays]]
name = "rpi_generic"
source = "muak-os/sbc-raspberrypi"
repository = "sbc/raspberrypi"
tag = "v0.4.0"
digest = "sha256:5555"
"#;

    /// Parses a fixture document of the given kind.
    fn document(kind: Kind, body: &str) -> Document {
        from_toml(kind, body.as_bytes(), RELEASE).expect("parse fixture document")
    }

    fn extensions_document() -> Document {
        document(Kind::Extensions, EXTENSIONS)
    }

    fn overlays_document() -> Document {
        document(Kind::Overlays, OVERLAYS)
    }

    fn profile(overlay: Option<OverlaySpec>, extensions: &[&str]) -> Profile {
        let customization =
            CustomizationSpec::new(extensions.iter().map(|name| (*name).to_owned()).collect())
                .expect("customization");
        let kernel = KernelSpec::new("muak-os/linux".into()).expect("kernel");

        Profile::new(overlay, customization, kernel)
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
    fn match_extensions_resolves_by_canonical_name() {
        // ARRANGE
        let document = extensions_document();
        let profile = profile(None, &["muak-os/qemu"]);
        let mut inputs = Vec::new();

        // ACT
        let extensions = match_extensions(&document, &profile, "ghcr.io/muak-os", &mut inputs)
            .expect("match extensions");

        // ASSERT
        let ext = extensions.first().expect("extension");
        assert_eq!(ext.name(), "muak-os/qemu");
        assert_eq!(ext.source(), "ghcr.io/muak-os/extensions/qemu@sha256:4444");
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs.first().expect("input").digest, "sha256:4444");
    }

    #[test]
    fn match_extensions_aliases_bare_names() {
        // ARRANGE
        let document = extensions_document();
        let profile = profile(None, &["qemu"]);

        // ACT
        let extensions = match_extensions(&document, &profile, "ghcr.io/muak-os", &mut Vec::new())
            .expect("match extensions");

        // ASSERT
        assert_eq!(
            extensions.first().expect("extension").name(),
            "muak-os/qemu"
        );
    }

    #[test]
    fn match_extensions_rejects_unknown_names() {
        // ARRANGE
        let document = extensions_document();
        let profile = profile(None, &["custom/thing"]);

        // ACT / ASSERT
        let error = match_extensions(&document, &profile, "ghcr.io/muak-os", &mut Vec::new())
            .expect_err("unknown extension should fail");
        assert!(error.to_string().contains("does not contain extension"));
    }

    #[test]
    fn match_overlay_resolves_by_name() {
        // ARRANGE
        let document = overlays_document();
        let spec = OverlaySpec::new("rpi_generic".into()).expect("overlay spec");
        let mut inputs = Vec::new();

        // ACT
        let overlay = match_overlay(
            &document,
            &spec,
            Arch::Amd64,
            "ghcr.io/muak-os",
            &mut inputs,
        )
        .expect("match overlay")
        .expect("overlay present");

        // ASSERT
        assert_eq!(overlay.name(), "rpi_generic");
        assert_eq!(overlay.image(), "muak-os/sbc-raspberrypi");
        assert_eq!(
            overlay.source_ref(),
            "ghcr.io/muak-os/sbc/raspberrypi@sha256:5555"
        );
        assert_eq!(inputs.first().expect("input").digest, "sha256:5555");
    }

    #[test]
    fn match_overlay_rejects_unknown_names() {
        // ARRANGE
        let document = overlays_document();
        let spec = OverlaySpec::new("board_x".into()).expect("overlay spec");

        // ACT / ASSERT
        let error = match_overlay(
            &document,
            &spec,
            Arch::Amd64,
            "ghcr.io/muak-os",
            &mut Vec::new(),
        )
        .expect_err("unknown overlay should fail");
        assert!(error.to_string().contains("does not contain overlay"));
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

    #[test]
    fn pinned_reference_is_registry_scoped_and_digest_addressed() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(
            pinned_reference("localhost:5000", "linux", "sha256:1111"),
            "localhost:5000/linux@sha256:1111"
        );
    }
}
