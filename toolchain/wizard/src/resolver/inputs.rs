//! Selection of extension and overlay inputs from their catalogs.

use kata::schema::documents::Document;
use kata::schema::entries::NamedEntry;
use kata::schema::view::{self, Role};
use koci::arch::Arch;

use super::catalog::pinned_reference;
use crate::domain::identity::ResolvedInput;
use crate::domain::input::{Extension, Overlay};
use crate::domain::profile::{OverlaySpec, Profile};
use crate::error::{Result, WizardError};

pub(super) fn extensions(
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

pub(super) fn overlay(
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

#[cfg(test)]
mod tests {
    use kata::schema::kinds::Kind;
    use kata::schema::parse::from_toml;

    use super::*;
    use crate::domain::profile::{CustomizationSpec, KernelSpec};

    const RELEASE: &str = "v1.2.3";

    const EXTENSIONS: &str = r#"api_version = "muak.dev/catalog/extensions/v1"
release = "v1.2.3"

[[extensions]]
name = "qemu"
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
    fn match_extensions_resolves_by_exact_name() {
        // ARRANGE
        let document = extensions_document();
        let profile = profile(None, &["qemu"]);
        let mut inputs = Vec::new();

        // ACT
        let extensions = extensions(&document, &profile, "ghcr.io/muak-os", &mut inputs)
            .expect("match extensions");

        // ASSERT
        let ext = extensions.first().expect("extension");
        assert_eq!(ext.name(), "qemu");
        assert_eq!(ext.source(), "ghcr.io/muak-os/extensions/qemu@sha256:4444");
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs.first().expect("input").digest, "sha256:4444");
    }

    #[test]
    fn match_extensions_rejects_unknown_names() {
        // ARRANGE
        let document = extensions_document();
        let profile = profile(None, &["custom/thing"]);

        // ACT / ASSERT
        let error = extensions(&document, &profile, "ghcr.io/muak-os", &mut Vec::new())
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
        let overlay = overlay(
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
        let error = overlay(
            &document,
            &spec,
            Arch::Amd64,
            "ghcr.io/muak-os",
            &mut Vec::new(),
        )
        .expect_err("unknown overlay should fail");
        assert!(error.to_string().contains("does not contain overlay"));
    }
}
