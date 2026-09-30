//! Resolution of selected tags to registry digests and their application.

use super::merge::Bases;
use super::plan::{Action, Auto, PlannedEntry, Selections};
use crate::error::{KataError, Result};
use crate::schema::documents::{ExtensionDocument, OverlayDocument};
use crate::schema::entries::NamedEntry;

/// Resolve the tag of every selected or auto-bumped entry to a digest.
///
/// # Errors
///
/// Returns an error when a registry resolution or auto discovery fails.
pub(crate) fn resolve_pins(
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

/// A resolved `identity → tag + digest` pin of the composition.
pub(crate) struct Pinned {
    pub(crate) identity: String,
    pub(crate) tag: String,
    pub(crate) digest: String,
}

/// Write every pin into its document entry.
///
/// # Errors
///
/// Returns an error when a pin references an unknown identity.
pub(crate) fn apply_pins(bases: &mut Bases, pins: &[Pinned]) -> Result<()> {
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

/// Mirror the pins into the planned entries for reporting.
pub(crate) fn apply_entries(entries: &mut [PlannedEntry], pins: &[Pinned]) {
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
