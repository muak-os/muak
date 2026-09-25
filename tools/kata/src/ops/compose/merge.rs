//! Merging carried and scratch documents into composed base documents.

use alloc::collections::BTreeMap;

use super::plan::Action;
use crate::schema::documents::{CoreDocument, ExtensionDocument, OverlayDocument};
use crate::schema::entries::NamedEntry;

/// Per-kind base documents a composition starts from.
#[derive(Debug, Default)]
pub(crate) struct Bases {
    pub(crate) core: Option<CoreDocument>,
    pub(crate) overlays: Option<OverlayDocument>,
    pub(crate) extensions: Option<ExtensionDocument>,
}

/// Where each base entry came from; selections turn entries into pins.
pub(crate) type Origins = BTreeMap<String, Action>;

/// A document whose named entries can be merged.
pub(crate) trait NamedEntries {
    fn named_entries(&self) -> &[NamedEntry];
    fn push_named(&mut self, entry: NamedEntry);
}

impl NamedEntries for OverlayDocument {
    fn named_entries(&self) -> &[NamedEntry] {
        &self.overlays
    }

    fn push_named(&mut self, entry: NamedEntry) {
        self.overlays.push(entry);
    }
}

impl NamedEntries for ExtensionDocument {
    fn named_entries(&self) -> &[NamedEntry] {
        &self.extensions
    }

    fn push_named(&mut self, entry: NamedEntry) {
        self.extensions.push(entry);
    }
}

/// Assemble the base documents and record each entry's origin.
pub(crate) fn assemble(
    output: &Bases,
    carried: Option<&Bases>,
    fresh: bool,
    release: &str,
) -> (Bases, Origins) {
    let mut origins = Origins::new();
    let carried_core = carried.and_then(|bases| bases.core.as_ref());
    let carried_overlays = carried.and_then(|bases| bases.overlays.as_ref());
    let carried_extensions = carried.and_then(|bases| bases.extensions.as_ref());

    let bases = Bases {
        core: assemble_core(
            output.core.as_ref(),
            carried_core,
            fresh,
            release,
            &mut origins,
        ),
        overlays: assemble_named(
            output.overlays.as_ref(),
            carried_overlays,
            fresh,
            release,
            "overlays",
            &mut origins,
        ),
        extensions: assemble_named(
            output.extensions.as_ref(),
            carried_extensions,
            fresh,
            release,
            "extensions",
            &mut origins,
        ),
    };

    (bases, origins)
}

fn assemble_core(
    output: Option<&CoreDocument>,
    carried: Option<&CoreDocument>,
    fresh: bool,
    release: &str,
    origins: &mut Origins,
) -> Option<CoreDocument> {
    let mut base = base_document(output, carried, fresh, release)?;

    if !fresh {
        match (output.as_ref(), carried) {
            (None, Some(_carried)) => record_core(origins, &base, Action::Carry),
            (Some(_), Some(carried)) => fill_core(&mut base, carried, origins),
            _ => {}
        }
    }
    base.set_release(release.to_owned());

    Some(base)
}

fn assemble_named<T: Clone + NamedEntries + SetRelease>(
    output: Option<&T>,
    carried: Option<&T>,
    fresh: bool,
    release: &str,
    prefix: &str,
    origins: &mut Origins,
) -> Option<T> {
    let mut base = base_document(output, carried, fresh, release)?;

    if !fresh {
        match (output.as_ref(), carried) {
            (None, Some(carried)) => record_named(origins, carried, prefix, Action::Carry),
            (Some(_), Some(carried)) => fill_named(&mut base, carried, prefix, origins),
            _ => {}
        }
    }
    base.set_release(release.to_owned());

    Some(base)
}

fn fill_named<T: NamedEntries>(base: &mut T, carried: &T, prefix: &str, origins: &mut Origins) {
    for entry in missing_named(base, carried) {
        origins.insert(format!("{prefix}/{}", entry.name), Action::Carry);
        base.push_named(entry);
    }
}

fn record_named<T: NamedEntries>(
    origins: &mut Origins,
    document: &T,
    prefix: &str,
    action: Action,
) {
    for entry in document.named_entries() {
        origins.insert(format!("{prefix}/{}", entry.name), action);
    }
}

fn missing_named<T: NamedEntries>(base: &T, carried: &T) -> Vec<NamedEntry> {
    carried
        .named_entries()
        .iter()
        .filter(|entry| {
            !base
                .named_entries()
                .iter()
                .any(|existing| existing.name == entry.name)
        })
        .cloned()
        .collect()
}

fn base_document<T: Clone + SetRelease>(
    output: Option<&T>,
    carried: Option<&T>,
    fresh: bool,
    release: &str,
) -> Option<T> {
    if fresh {
        return None;
    }
    let mut base = output.cloned().or_else(|| carried.cloned())?;
    base.set_release(release.to_owned());

    Some(base)
}

fn record_core(origins: &mut Origins, core: &CoreDocument, action: Action) {
    for kernel in &core.kernels {
        origins.insert(format!("kernels/{}", kernel.source), action);
    }
    if core.stub.is_some() {
        origins.insert("stub".to_owned(), action);
    }
    if core.installer.is_some() {
        origins.insert("installer".to_owned(), action);
    }
}

fn fill_core(base: &mut CoreDocument, carried: &CoreDocument, origins: &mut Origins) {
    if base.stub.is_none()
        && let Some(stub) = carried.stub.clone()
    {
        origins.insert("stub".to_owned(), Action::Carry);
        base.stub = Some(stub);
    }
    if base.installer.is_none()
        && let Some(installer) = carried.installer.clone()
    {
        origins.insert("installer".to_owned(), Action::Carry);
        base.installer = Some(installer);
    }
    for kernel in &carried.kernels {
        let identity = format!("kernels/{}", kernel.source);
        if !base
            .kernels
            .iter()
            .any(|existing| existing.source == kernel.source)
        {
            origins.insert(identity, Action::Carry);
            base.kernels.push(kernel.clone());
        }
    }
}

trait SetRelease {
    fn set_release(&mut self, release: String);
}

impl SetRelease for CoreDocument {
    fn set_release(&mut self, release: String) {
        self.release = release;
    }
}

impl SetRelease for OverlayDocument {
    fn set_release(&mut self, release: String) {
        self.release = release;
    }
}

impl SetRelease for ExtensionDocument {
    fn set_release(&mut self, release: String) {
        self.release = release;
    }
}

#[cfg(test)]
mod tests {
    use super::{Bases, assemble};
    use crate::schema::documents::{CoreDocument, OverlayDocument};
    use crate::schema::entries::{NamedEntry, SourcedEntry};
    use crate::schema::kinds::{CORE_API_VERSION, OVERLAYS_API_VERSION};

    fn core(kernels: &[&str], stub: bool) -> CoreDocument {
        CoreDocument {
            api_version: CORE_API_VERSION.to_owned(),
            release: "v1.0.0".to_owned(),
            kernels: kernels
                .iter()
                .map(|source| SourcedEntry {
                    source: (*source).to_owned(),
                    repository: "linux".to_owned(),
                    tag: "v6".to_owned(),
                    digest: format!("sha256:{source}"),
                })
                .collect(),
            stub: stub.then(|| SourcedEntry {
                source: "muak-os/stub".to_owned(),
                repository: "stub".to_owned(),
                tag: "v0".to_owned(),
                digest: "sha256:stub".to_owned(),
            }),
            installer: None,
        }
    }

    fn overlays(names: &[&str]) -> OverlayDocument {
        OverlayDocument {
            api_version: OVERLAYS_API_VERSION.to_owned(),
            release: "v1.0.0".to_owned(),
            overlays: names
                .iter()
                .map(|name| NamedEntry {
                    name: (*name).to_owned(),
                    source: "muak-os/sbc-raspberrypi".to_owned(),
                    repository: "sbc/raspberrypi".to_owned(),
                    tag: "v0".to_owned(),
                    digest: format!("sha256:{name}"),
                })
                .collect(),
        }
    }

    #[test]
    fn assemble_carries_missing_entries_into_scratch_documents() {
        // ARRANGE
        let output = Bases {
            core: Some(core(&["muak-os/linux"], false)),
            overlays: None,
            extensions: None,
        };
        let carried = Bases {
            core: Some(core(&["muak-os/linux", "muak-os/rt"], true)),
            overlays: Some(overlays(&["rpi_generic"])),
            extensions: None,
        };

        // ACT
        let (bases, origins) = assemble(&output, Some(&carried), false, "v1.1.0");

        // ASSERT
        let core = bases.core.expect("core base");
        assert_eq!(core.release, "v1.1.0");
        assert_eq!(core.kernels.len(), 2, "scratch pin kept, carried pin added");
        assert!(
            core.kernels
                .iter()
                .any(|kernel| kernel.source == "muak-os/linux")
        );
        assert!(
            core.kernels
                .iter()
                .any(|kernel| kernel.source == "muak-os/rt")
        );
        assert!(core.stub.is_some(), "stub is carried");
        let overlays = bases.overlays.expect("overlays base");
        assert_eq!(
            overlays.overlays.first().expect("overlay").name,
            "rpi_generic"
        );
        assert_eq!(
            origins.get("overlays/rpi_generic"),
            Some(&super::Action::Carry)
        );
        assert_eq!(origins.get("stub"), Some(&super::Action::Carry));
    }

    #[test]
    fn assemble_ignores_carried_documents_when_fresh() {
        // ARRANGE
        let carried = Bases {
            core: Some(core(&["muak-os/linux"], true)),
            overlays: Some(overlays(&["rpi_generic"])),
            extensions: None,
        };

        // ACT
        let (bases, origins) = assemble(&Bases::default(), Some(&carried), true, "v1.1.0");

        // ASSERT
        assert!(bases.core.is_none());
        assert!(bases.overlays.is_none());
        assert!(origins.is_empty());
    }

    #[test]
    fn assembled_documents_expose_the_document_enum() {
        // ARRANGE
        let output = Bases {
            core: Some(core(&["muak-os/linux"], false)),
            overlays: Some(overlays(&["rpi_generic"])),
            extensions: None,
        };

        // ACT
        let (bases, _) = assemble(&output, None, false, "v1.1.0");

        // ASSERT
        assert!(bases.core.is_some());
        assert!(bases.overlays.is_some());
    }
}
