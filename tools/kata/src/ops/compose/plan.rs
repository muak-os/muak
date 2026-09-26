//! Composition plans selections applied to base documents.

use alloc::collections::BTreeMap;

use super::merge::Bases;
use crate::error::{KataError, Result};
use crate::schema::documents::{Document, ExtensionDocument, OverlayDocument};
use crate::schema::entries::{NamedEntry, SourcedEntry};
use crate::schema::kinds::Kind;

/// Payload tag selections for one composition, addressed by role shorthand
/// (`kernel`, `stub`, `installer`) or by entry identity (`kernels/SOURCE`,
/// `overlays/NAME`, `extensions/NAME`).
#[derive(Debug, Default, Clone)]
pub struct Selections {
    /// `KEY=TAG` pairs; see the type documentation for the `KEY` forms.
    pub sets: Vec<(String, String)>,
}

/// Auto-selection policy: newest-tag discovery for one repository.
pub(crate) type Auto<'a> = dyn Fn(&str) -> Result<String> + 'a;

impl Selections {
    /// The selected tag for `identity`, if any selection addresses it. The
    /// last selection for a key wins.
    pub(crate) fn tag_of(&self, identity: &str) -> Option<&String> {
        self.sets
            .iter()
            .rev()
            .find(|pair| selection_matches(&pair.0, identity))
            .map(|pair| &pair.1)
    }
}

fn selection_matches(key: &str, identity: &str) -> bool {
    if key == "kernel" {
        return identity.starts_with("kernels/");
    }

    identity == key
}

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

impl Action {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Pin => "pin",
            Self::Keep => "keep",
            Self::Carry => "carry",
        }
    }
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
    pub(crate) release: String,
    pub(crate) entries: Vec<PlannedEntry>,
    pub(crate) documents: Vec<Document>,
}

/// Build the plan: merge bases, apply selections, resolve pinned digests.
///
/// # Errors
///
/// Returns an error when a selection references an unknown entry, auto
/// discovery fails, or a registry resolution fails.
pub(crate) fn build(
    bases: &mut Bases,
    origins: &BTreeMap<String, Action>,
    selections: &Selections,
    release: &str,
    resolve: &dyn Fn(&str, &str) -> Result<String>,
    auto: Option<&Auto<'_>>,
) -> Result<Plan> {
    let mut entries = enumerate(bases, origins);
    validate_selections(selections, &entries)?;
    let pins = resolve_pins(selections, &entries, resolve, auto)?;
    apply_pins(bases, &pins)?;
    apply_entries(&mut entries, &pins);
    let documents = take_documents(bases);

    Ok(Plan {
        release: release.to_owned(),
        entries,
        documents,
    })
}

/// Print the plan as JSON on stdout and a table on stderr.
///
/// # Errors
///
/// Returns an error when the plan fails to serialize.
pub(crate) fn print(plan: &Plan) -> Result<()> {
    let entries: Vec<serde_json::Value> = plan
        .entries
        .iter()
        .map(|entry| {
            serde_json::json!({
                "identity": entry.identity,
                "kind": entry.kind.dir(),
                "name": entry.name,
                "source": entry.source,
                "repository": entry.repository,
                "tag": entry.tag,
                "digest": entry.digest,
                "action": entry.action.as_str(),
            })
        })
        .collect();
    let document = serde_json::json!({ "release": plan.release, "entries": entries });
    let rendered = serde_json::to_string_pretty(&document)
        .map_err(|error| KataError::Document(format!("Failed to serialize plan: {error}")))?;
    println!("{rendered}");

    for entry in &plan.entries {
        eprintln!(
            "{:>6}  {}  {} ({})",
            entry.action.as_str(),
            entry.identity,
            entry.digest,
            entry.tag
        );
    }

    Ok(())
}

fn enumerate(bases: &Bases, origins: &BTreeMap<String, Action>) -> Vec<PlannedEntry> {
    let mut entries = Vec::new();

    if let Some(ref core) = bases.core {
        for kernel in &core.kernels {
            let identity = format!("kernels/{}", kernel.source);
            entries.push(planned(identity, Kind::Core, None, kernel, origins));
        }
        if let Some(stub) = core.stub.as_ref() {
            entries.push(planned("stub".to_owned(), Kind::Core, None, stub, origins));
        }
        if let Some(installer) = core.installer.as_ref() {
            entries.push(planned(
                "installer".to_owned(),
                Kind::Core,
                None,
                installer,
                origins,
            ));
        }
    }
    if let Some(ref overlays) = bases.overlays {
        for overlay in &overlays.overlays {
            let identity = format!("overlays/{}", overlay.name);
            let view = SourcedEntry {
                source: overlay.source.clone(),
                repository: overlay.repository.clone(),
                tag: overlay.tag.clone(),
                digest: overlay.digest.clone(),
            };
            entries.push(planned(
                identity,
                Kind::Overlays,
                Some(overlay.name.clone()),
                &view,
                origins,
            ));
        }
    }
    if let Some(ref extensions) = bases.extensions {
        for extension in &extensions.extensions {
            let identity = format!("extensions/{}", extension.name);
            let view = SourcedEntry {
                source: extension.source.clone(),
                repository: extension.repository.clone(),
                tag: extension.tag.clone(),
                digest: extension.digest.clone(),
            };
            entries.push(planned(
                identity,
                Kind::Extensions,
                Some(extension.name.clone()),
                &view,
                origins,
            ));
        }
    }

    entries
}

fn planned(
    identity: String,
    kind: Kind,
    name: Option<String>,
    entry: &SourcedEntry,
    origins: &BTreeMap<String, Action>,
) -> PlannedEntry {
    let action = origins.get(&identity).copied().unwrap_or(Action::Keep);

    PlannedEntry {
        identity,
        kind,
        name,
        source: entry.source.clone(),
        repository: entry.repository.clone(),
        tag: entry.tag.clone(),
        digest: entry.digest.clone(),
        action,
    }
}

/// Validate that every selection matches a planned entry of the right role.
///
/// # Errors
///
/// Returns an error when a core role or named entry has no planned entry to
/// retag.
fn validate_selections(selections: &Selections, entries: &[PlannedEntry]) -> Result<()> {
    let mut missing: Vec<String> = Vec::new();

    for pair in &selections.sets {
        if !entries
            .iter()
            .any(|entry| selection_matches(&pair.0, &entry.identity))
        {
            missing.push(selection_missing_message(&pair.0));
        }
    }

    if missing.is_empty() {
        return Ok(());
    }

    Err(KataError::Document(format!(
        "selections without a matching entry: {}",
        missing.join(", ")
    )))
}

fn selection_missing_message(key: &str) -> String {
    match key {
        "kernel" => "kernel (no kernels entry to retag)".to_owned(),
        "stub" => "stub (no stub entry to retag)".to_owned(),
        "installer" => "installer (no installer entry to retag)".to_owned(),
        _ => {
            let name = key.rsplit('/').next().unwrap_or("?");
            format!("'{name}' (introduce it with `kata add`)")
        }
    }
}

fn resolve_pins(
    selections: &Selections,
    entries: &[PlannedEntry],
    resolve: &dyn Fn(&str, &str) -> Result<String>,
    auto: Option<&Auto<'_>>,
) -> Result<Vec<Pinned>> {
    let mut pins = Vec::new();

    for entry in entries {
        let tag = match selections.tag_of(&entry.identity) {
            Some(tag) => tag.clone(),
            None => match auto {
                Some(auto) => auto(&entry.repository)?,
                None => continue,
            },
        };
        let digest = resolve(&entry.repository, &tag)?;
        pins.push(Pinned {
            identity: entry.identity.clone(),
            tag: tag.clone(),
            digest,
        });
    }

    Ok(pins)
}

struct Pinned {
    identity: String,
    tag: String,
    digest: String,
}

fn apply_pins(bases: &mut Bases, pins: &[Pinned]) -> Result<()> {
    for pin in pins {
        if pin.identity == "stub" {
            retag_core_slot(bases, "stub", pin)?;
        } else if pin.identity == "installer" {
            retag_core_slot(bases, "installer", pin)?;
        } else if let Some(("kernels", source)) = pin.identity.split_once('/') {
            retag_kernel(bases, source, pin)?;
        } else if let Some(("overlays", name)) = pin.identity.split_once('/') {
            retag_named(bases.overlays.as_mut(), name, pin)?;
        } else if let Some(("extensions", name)) = pin.identity.split_once('/') {
            retag_named(bases.extensions.as_mut(), name, pin)?;
        } else {
            return Err(KataError::Document(format!(
                "unknown entry identity '{}'",
                pin.identity
            )));
        }
    }

    Ok(())
}

fn apply_entries(entries: &mut [PlannedEntry], pins: &[Pinned]) {
    for pin in pins {
        let Some(entry) = entries
            .iter_mut()
            .find(|entry| entry.identity == pin.identity)
        else {
            continue;
        };
        entry.tag.clone_from(&pin.tag);
        entry.digest.clone_from(&pin.digest);
        entry.action = Action::Pin;
    }
}

fn retag_kernel(bases: &mut Bases, source: &str, pin: &Pinned) -> Result<()> {
    let Some(core) = bases.core.as_mut() else {
        return Err(KataError::Document("no core document to retag".to_owned()));
    };
    let Some(kernel) = core
        .kernels
        .iter_mut()
        .find(|kernel| kernel.source == source)
    else {
        return Err(KataError::Document(format!(
            "no kernels entry for source '{source}'"
        )));
    };

    kernel.tag.clone_from(&pin.tag);
    kernel.digest.clone_from(&pin.digest);

    Ok(())
}

fn retag_core_slot(bases: &mut Bases, role: &str, pin: &Pinned) -> Result<()> {
    let Some(core) = bases.core.as_mut() else {
        return Err(KataError::Document("no core document to retag".to_owned()));
    };
    let slot = match role {
        "stub" => &mut core.stub,
        _ => &mut core.installer,
    };
    let Some(entry) = slot.as_mut() else {
        return Err(KataError::Document(format!("no {role} entry to retag")));
    };

    entry.tag.clone_from(&pin.tag);
    entry.digest.clone_from(&pin.digest);

    Ok(())
}

fn retag_named<T: NamedEntries>(document: Option<&mut T>, name: &str, pin: &Pinned) -> Result<()> {
    let Some(document) = document else {
        return Err(KataError::Document(format!(
            "no {} document to retag",
            pin.identity
                .split_once('/')
                .map_or("named", |(kind, _)| kind)
        )));
    };
    let Some(entry) = document
        .named_entries_mut()
        .iter_mut()
        .find(|entry| entry.name == name)
    else {
        return Err(KataError::Document(format!(
            "no entry named '{name}' to retag"
        )));
    };

    entry.tag.clone_from(&pin.tag);
    entry.digest.clone_from(&pin.digest);

    Ok(())
}

trait NamedEntries {
    fn named_entries_mut(&mut self) -> &mut [NamedEntry];
}

impl NamedEntries for OverlayDocument {
    fn named_entries_mut(&mut self) -> &mut [NamedEntry] {
        &mut self.overlays
    }
}

impl NamedEntries for ExtensionDocument {
    fn named_entries_mut(&mut self) -> &mut [NamedEntry] {
        &mut self.extensions
    }
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

    #[test]
    fn selections_pin_their_entries_and_carry_the_rest() {
        // ARRANGE
        let mut bases = bases();
        let origins = BTreeMap::from([
            ("kernels/muak-os/linux".to_owned(), Action::Carry),
            ("stub".to_owned(), Action::Carry),
            ("installer".to_owned(), Action::Carry),
            ("overlays/rpi_generic".to_owned(), Action::Carry),
        ]);
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
    fn selections_reject_unknown_named_entries() {
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
        assert!(
            error
                .to_string()
                .contains("'rpi4b' (introduce it with `kata add`)")
        );
    }

    #[test]
    fn auto_bumps_entries_without_a_selection() {
        // ARRANGE
        let mut bases = bases();
        let origins = BTreeMap::from([
            ("kernels/muak-os/linux".to_owned(), Action::Carry),
            ("stub".to_owned(), Action::Carry),
            ("installer".to_owned(), Action::Carry),
            ("overlays/rpi_generic".to_owned(), Action::Carry),
        ]);
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
