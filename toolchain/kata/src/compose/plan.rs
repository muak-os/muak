//! Composition plans selections applied to base documents.

use alloc::collections::BTreeMap;

use super::apply::{apply_entries, apply_pins};
use super::entries::enumerate;
use super::introductions::{classify, ensure_documents};
use super::merge::Bases;
use super::pins::resolve_pins;
use super::selection;
use crate::error::Result;
use crate::schema::documents::Document;
use crate::schema::kinds::Kind;

/// Payload tag selections for one composition, addressed by role shorthand
/// (`kernel`, `stub`, `installer`) or by entry identity (`kernels/SOURCE`,
/// `overlays/NAME`, `extensions/NAME`).
#[derive(Debug, Default, Clone)]
pub struct Selections {
    /// `KEY=VALUE` pairs; see the type documentation for the forms.
    pub sets: Vec<(String, String)>,
}

/// Auto-selection policy: newest-tag discovery for one repository.
pub(crate) type Auto<'a> = dyn Fn(&str) -> Result<String> + 'a;

/// Why an entry is part of the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// The tag was selected this run and the digest freshly resolved.
    Pin,
    /// The entry came from the scratch documents of the output root.
    Keep,
    /// The entry was carried verbatim from the lineage parent line.
    Carry,
}

/// One planned entry of the composed line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlannedEntry {
    pub(crate) identity: String,
    pub(crate) kind: Kind,
    pub(crate) name: Option<String>,
    pub(crate) source: String,
    pub(crate) repository: String,
    pub(crate) tag: String,
    pub(crate) digest: String,
    pub(crate) action: Action,
}

/// The resolved composition: final documents plus their planned entries.
#[derive(Debug)]
pub(crate) struct Plan {
    pub(crate) entries: Vec<PlannedEntry>,
    pub(crate) documents: Vec<Document>,
}

/// Build the plan: merge bases, apply selections, resolve pinned digests.
///
/// # Errors
///
/// Returns an error when a selection is malformed, references an entry it
/// cannot retag or introduce, auto discovery fails, or a registry resolution
/// fails.
pub(crate) fn build(
    bases: &mut Bases,
    origins: &BTreeMap<String, Action>,
    selections: &Selections,
    release: &str,
    resolve: &dyn Fn(&str, &str) -> Result<String>,
    auto: Option<&Auto<'_>>,
) -> Result<Plan> {
    let selections = selection::parse_selections(&selections.sets)?;
    let mut entries = enumerate(bases, origins);
    let introductions = classify(&selections, &entries)?;
    ensure_documents(bases, &introductions, release);
    entries.extend(
        introductions
            .iter()
            .map(super::introductions::Introduction::planned),
    );
    let pins = resolve_pins(&selections, &entries, resolve, auto)?;
    apply_pins(bases, &pins)?;
    apply_entries(&mut entries, &pins);
    let documents = take_documents(bases);

    Ok(Plan { entries, documents })
}

fn take_documents(bases: &mut Bases) -> Vec<Document> {
    let mut documents = Vec::new();
    if let Some(core) = bases.core.take() {
        documents.push(Document::Core(core));
    }
    if let Some(overlays) = bases.overlays.take() {
        documents.push(Document::Overlays(overlays));
    }
    if let Some(extensions) = bases.extensions.take() {
        documents.push(Document::Extensions(extensions));
    }

    documents
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;

    use super::{Action, Bases, Selections, build};
    use crate::schema::documents::{CoreDocument, Document, OverlayDocument};
    use crate::schema::entries::{NamedEntry, SourcedEntry};
    use crate::schema::kinds::{CORE_API_VERSION, OVERLAYS_API_VERSION};

    fn sourced(source: &str, repository: &str, tag: &str, digest: &str) -> SourcedEntry {
        SourcedEntry {
            source: source.to_owned(),
            repository: repository.to_owned(),
            tag: tag.to_owned(),
            digest: digest.to_owned(),
        }
    }

    fn named(name: &str) -> NamedEntry {
        NamedEntry {
            name: name.to_owned(),
            source: "muak-os/sbc-raspberrypi".to_owned(),
            repository: "sbc/raspberrypi".to_owned(),
            tag: "v0".to_owned(),
            digest: format!("sha256:{name}"),
        }
    }

    fn bases() -> Bases {
        Bases {
            core: Some(CoreDocument {
                api_version: CORE_API_VERSION.to_owned(),
                release: "v1.0.0".to_owned(),
                kernels: vec![sourced("muak-os/linux", "linux", "v6", "sha256:old-kernel")],
                stub: Some(sourced("muak-os/stub", "stub", "v0", "sha256:old-stub")),
                installer: Some(sourced(
                    "muak-os/muak",
                    "installer",
                    "v1",
                    "sha256:old-installer",
                )),
            }),
            overlays: Some(OverlayDocument {
                api_version: OVERLAYS_API_VERSION.to_owned(),
                release: "v1.0.0".to_owned(),
                overlays: vec![named("rpi_generic")],
            }),
            extensions: None,
        }
    }

    fn fake_resolve(repository: &str, tag: &str) -> String {
        format!("sha256:{repository}@{tag}")
    }

    fn origins_all_carried() -> BTreeMap<String, Action> {
        BTreeMap::from([
            ("kernels/muak-os/linux".to_owned(), Action::Carry),
            ("stub".to_owned(), Action::Carry),
            ("installer".to_owned(), Action::Carry),
            ("overlays/rpi_generic".to_owned(), Action::Carry),
        ])
    }

    #[test]
    fn selections_pin_their_entries_and_carry_the_rest() {
        // ARRANGE
        let mut bases = bases();
        let origins = origins_all_carried();
        let selections = Selections {
            sets: vec![
                ("installer".to_owned(), "v2".to_owned()),
                ("overlays/rpi_generic".to_owned(), "v9".to_owned()),
            ],
        };

        // ACT
        let plan = build(
            &mut bases,
            &origins,
            &selections,
            "v1.1.0",
            &|repository, tag| Ok(fake_resolve(repository, tag)),
            None,
        )
        .expect("build plan");

        // ASSERT
        let Document::Core(ref core) = *plan.documents.first().expect("core document") else {
            panic!("expected core document");
        };
        assert_eq!(
            core.kernels.first().expect("kernel").tag,
            "v6",
            "unselected entry is carried"
        );
        assert_eq!(core.installer.as_ref().expect("installer").tag, "v2");
        assert_eq!(
            core.installer.as_ref().expect("installer").digest,
            "sha256:installer@v2"
        );
        let installer_entry = plan
            .entries
            .iter()
            .find(|entry| entry.identity == "installer")
            .expect("installer entry");
        assert_eq!(installer_entry.action, Action::Pin);
        let kernel_entry = plan
            .entries
            .iter()
            .find(|entry| entry.identity == "kernels/muak-os/linux")
            .expect("kernel entry");
        assert_eq!(kernel_entry.action, Action::Carry);
        let overlay_entry = plan
            .entries
            .iter()
            .find(|entry| entry.identity == "overlays/rpi_generic")
            .expect("overlay entry");
        assert_eq!(overlay_entry.action, Action::Pin);
        assert_eq!(overlay_entry.digest, "sha256:sbc/raspberrypi@v9");
    }

    #[test]
    fn selections_reject_unknown_named_entries_without_a_source() {
        // ARRANGE
        let mut bases = bases();
        let origins = BTreeMap::new();
        let selections = Selections {
            sets: vec![("overlays/rpi4b".to_owned(), "v9".to_owned())],
        };

        // ACT
        let error = build(
            &mut bases,
            &origins,
            &selections,
            "v1.1.0",
            &|repository, tag| Ok(fake_resolve(repository, tag)),
            None,
        )
        .expect_err("unknown overlay must fail");

        // ASSERT
        assert!(error.to_string().contains("has no entry to retag"));
        assert!(error.to_string().contains("SOURCE@REPOSITORY@TAG"));
    }

    #[test]
    fn selections_introduce_named_entries_with_a_source() {
        // ARRANGE
        let mut bases = bases();
        let origins = BTreeMap::new();
        let selections = Selections {
            sets: vec![(
                "overlays/rpi4b".to_owned(),
                "muak-os/sbc-raspberrypi@sbc/raspberrypi@v0.4.0".to_owned(),
            )],
        };

        // ACT
        let plan = build(
            &mut bases,
            &origins,
            &selections,
            "v1.1.0",
            &|repository, tag| Ok(fake_resolve(repository, tag)),
            None,
        )
        .expect("build plan");

        // ASSERT
        let Document::Overlays(ref overlays) = *plan.documents.last().expect("overlays document")
        else {
            panic!("expected overlays document");
        };
        assert_eq!(overlays.overlays.len(), 2);
        let rpi4b = overlays
            .overlays
            .iter()
            .find(|entry| entry.name == "rpi4b")
            .expect("introduced overlay");
        assert_eq!(rpi4b.source, "muak-os/sbc-raspberrypi");
        assert_eq!(rpi4b.repository, "sbc/raspberrypi");
        assert_eq!(rpi4b.tag, "v0.4.0");
        assert_eq!(rpi4b.digest, "sha256:sbc/raspberrypi@v0.4.0");
    }

    #[test]
    fn selections_introduce_a_whole_fresh_core_line() {
        // ARRANGE
        let mut bases = Bases::default();
        let origins = BTreeMap::new();
        let selections = Selections {
            sets: vec![
                ("kernels/muak-os/linux".to_owned(), "linux@v6".to_owned()),
                ("stub".to_owned(), "muak-os/stub@stub@v1".to_owned()),
                (
                    "installer".to_owned(),
                    "muak-os/muak@installer@v1".to_owned(),
                ),
            ],
        };

        // ACT
        let plan = build(
            &mut bases,
            &origins,
            &selections,
            "v1.0.0",
            &|repository, tag| Ok(fake_resolve(repository, tag)),
            None,
        )
        .expect("build plan");

        // ASSERT
        assert_eq!(plan.entries.len(), 3);
        assert!(plan.entries.iter().all(|entry| entry.action == Action::Pin));
        let Document::Core(ref core) = *plan.documents.first().expect("core document") else {
            panic!("expected core document");
        };
        assert_eq!(core.release, "v1.0.0");
        let kernel = core.kernels.first().expect("introduced kernel");
        assert_eq!(kernel.source, "muak-os/linux");
        assert_eq!(kernel.repository, "linux");
        assert_eq!(kernel.tag, "v6");
        assert_eq!(
            core.stub.as_ref().expect("introduced stub").source,
            "muak-os/stub"
        );
        assert_eq!(core.stub.as_ref().expect("introduced stub").tag, "v1");
        assert!(
            core.installer
                .as_ref()
                .expect("introduced installer")
                .source
                .starts_with("muak-os/muak")
        );
    }

    #[test]
    fn retag_in_selections_update_the_repository() {
        // ARRANGE
        let mut bases = bases();
        let origins = origins_all_carried();
        let selections = Selections {
            sets: vec![("kernels/muak-os/linux".to_owned(), "linux-rt@v7".to_owned())],
        };

        // ACT
        let plan = build(
            &mut bases,
            &origins,
            &selections,
            "v1.1.0",
            &|repository, tag| Ok(fake_resolve(repository, tag)),
            None,
        )
        .expect("build plan");

        // ASSERT
        let Document::Core(ref core) = *plan.documents.first().expect("core document") else {
            panic!("expected core document");
        };
        let kernel = core.kernels.first().expect("kernel");
        assert_eq!(kernel.repository, "linux-rt");
        assert_eq!(kernel.tag, "v7");
        assert_eq!(kernel.source, "muak-os/linux", "the source is the identity");
    }

    #[test]
    fn auto_bumps_entries_without_a_selection() {
        // ARRANGE
        let mut bases = bases();
        let origins = origins_all_carried();
        let selections = Selections::default();

        // ACT
        let plan = build(
            &mut bases,
            &origins,
            &selections,
            "v1.1.0",
            &|repository, tag| Ok(fake_resolve(repository, tag)),
            Some(&|repository: &str| Ok(format!("v9-{repository}"))),
        )
        .expect("build plan");

        // ASSERT
        let Document::Core(ref core) = *plan.documents.first().expect("core document") else {
            panic!("expected core document");
        };
        assert_eq!(core.kernels.first().expect("kernel").tag, "v9-linux");
        assert_eq!(
            core.kernels.first().expect("kernel").digest,
            "sha256:linux@v9-linux"
        );
        let kernel_entry = plan
            .entries
            .iter()
            .find(|entry| entry.identity == "kernels/muak-os/linux")
            .expect("kernel entry");
        assert_eq!(kernel_entry.action, Action::Pin);
    }

    #[test]
    fn explicit_selections_beat_auto_discovery() {
        // ARRANGE
        let mut bases = bases();
        let origins = BTreeMap::new();
        let selections = Selections {
            sets: vec![("installer".to_owned(), "v2".to_owned())],
        };

        // ACT
        let plan = build(
            &mut bases,
            &origins,
            &selections,
            "v1.1.0",
            &|repository, tag| Ok(fake_resolve(repository, tag)),
            Some(&|_| Ok("v9".to_owned())),
        )
        .expect("build plan");

        // ASSERT
        let Document::Core(ref core) = *plan.documents.first().expect("core document") else {
            panic!("expected core document");
        };
        assert_eq!(core.installer.as_ref().expect("installer").tag, "v2");
        assert_eq!(
            core.stub.as_ref().expect("stub").tag,
            "v9",
            "auto fills unselected entries"
        );
    }

    #[test]
    fn duplicate_selections_pick_the_last_tag() {
        // ARRANGE
        let mut bases = bases();
        let origins = BTreeMap::new();
        let selections = Selections {
            sets: vec![
                ("installer".to_owned(), "v2".to_owned()),
                ("installer".to_owned(), "v3".to_owned()),
            ],
        };

        // ACT
        let plan = build(
            &mut bases,
            &origins,
            &selections,
            "v1.1.0",
            &|repository, tag| Ok(fake_resolve(repository, tag)),
            None,
        )
        .expect("build plan");

        // ASSERT
        let Document::Core(ref core) = *plan.documents.first().expect("core document") else {
            panic!("expected core document");
        };
        assert_eq!(core.installer.as_ref().expect("installer").tag, "v3");
        assert_eq!(
            core.installer.as_ref().expect("installer").digest,
            "sha256:installer@v3"
        );
    }
}
