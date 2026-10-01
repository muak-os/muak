//! Enumeration of a line's planned entries and selection classification.

use alloc::collections::BTreeMap;

use super::merge::Bases;
use super::plan::{Action, PlannedEntry};
use crate::schema::entries::SourcedEntry;
use crate::schema::kinds::Kind;

/// Enumerate every entry of the base documents as a planned entry.
pub(crate) fn enumerate(bases: &Bases, origins: &BTreeMap<String, Action>) -> Vec<PlannedEntry> {
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
