//! Application of resolved pins to the base documents.

use super::merge::Bases;
use super::pins::Pinned;
use super::plan::{Action, PlannedEntry};
use crate::error::{KataError, Result};
use crate::schema::documents::{ExtensionDocument, OverlayDocument};
use crate::schema::entries::{NamedEntry, SourcedEntry};

/// Write every pin into its document entry, inserting introductions.
///
/// # Errors
///
/// Returns an error when a pin references an unknown identity or an
/// introduction lacks its source or repository.
pub(crate) fn apply_pins(bases: &mut Bases, pins: &[Pinned]) -> Result<()> {
    for pin in pins {
        if pin.identity == "stub" {
            retag_slot(bases, "stub", pin)?;
        } else if pin.identity == "installer" {
            retag_slot(bases, "installer", pin)?;
        } else if let Some(("kernels", source)) = pin.identity.split_once('/') {
            retag_kernel(bases, source, pin)?;
        } else if let Some((dir, name)) = pin.identity.split_once('/') {
            retag_named(bases, dir, name, pin)?;
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

    match core
        .kernels
        .iter_mut()
        .find(|kernel| kernel.source == source)
    {
        Some(kernel) => {
            kernel.tag.clone_from(&pin.tag);
            kernel.digest.clone_from(&pin.digest);
            if let Some(ref repository) = pin.repository {
                kernel.repository.clone_from(repository);
            }
        }
        None => core.kernels.push(SourcedEntry {
            source: source.to_owned(),
            repository: pin.repository.clone().ok_or_else(|| {
                KataError::Document(format!(
                    "introducing '{}' requires a REPOSITORY@TAG value",
                    pin.identity
                ))
            })?,
            tag: pin.tag.clone(),
            digest: pin.digest.clone(),
        }),
    }

    Ok(())
}

fn retag_slot(bases: &mut Bases, role: &str, pin: &Pinned) -> Result<()> {
    let Some(core) = bases.core.as_mut() else {
        return Err(KataError::Document("no core document to retag".to_owned()));
    };
    let slot = match role {
        "stub" => &mut core.stub,
        _ => &mut core.installer,
    };

    match slot.as_mut() {
        Some(existing) => {
            existing.tag.clone_from(&pin.tag);
            existing.digest.clone_from(&pin.digest);
            if let Some(ref repository) = pin.repository {
                existing.repository.clone_from(repository);
            }
            if let Some(ref source) = pin.source {
                existing.source.clone_from(source);
            }
        }
        None => {
            *slot = Some(SourcedEntry {
                source: pin.source.clone().ok_or_else(|| {
                    KataError::Document(format!(
                        "introducing '{}' requires a SOURCE@REPOSITORY@TAG value",
                        pin.identity
                    ))
                })?,
                repository: pin.repository.clone().ok_or_else(|| {
                    KataError::Document(format!(
                        "introducing '{}' requires a REPOSITORY@TAG value",
                        pin.identity
                    ))
                })?,
                tag: pin.tag.clone(),
                digest: pin.digest.clone(),
            });
        }
    }

    Ok(())
}

fn retag_named(bases: &mut Bases, dir: &str, name: &str, pin: &Pinned) -> Result<()> {
    match dir {
        "overlays" => {
            let Some(overlays) = bases.overlays.as_mut() else {
                return Err(missing_document(dir, pin));
            };
            retag_named_entries(overlays, name, pin)
        }
        "extensions" => {
            let Some(extensions) = bases.extensions.as_mut() else {
                return Err(missing_document(dir, pin));
            };
            retag_named_entries(extensions, name, pin)
        }
        _ => Err(KataError::Document(format!(
            "unknown entry identity '{}'",
            pin.identity
        ))),
    }
}

fn retag_named_entries<T: NamedEntries>(document: &mut T, name: &str, pin: &Pinned) -> Result<()> {
    match document
        .named_entries_mut()
        .iter_mut()
        .find(|entry| entry.name == name)
    {
        Some(existing) => {
            existing.tag.clone_from(&pin.tag);
            existing.digest.clone_from(&pin.digest);
            if let Some(ref repository) = pin.repository {
                existing.repository.clone_from(repository);
            }
            if let Some(ref source) = pin.source {
                existing.source.clone_from(source);
            }
        }
        None => document.push_named(NamedEntry {
            name: name.to_owned(),
            source: pin.source.clone().ok_or_else(|| {
                KataError::Document(format!(
                    "introducing '{}' requires a SOURCE@REPOSITORY@TAG value",
                    pin.identity
                ))
            })?,
            repository: pin.repository.clone().ok_or_else(|| {
                KataError::Document(format!(
                    "introducing '{}' requires a REPOSITORY@TAG value",
                    pin.identity
                ))
            })?,
            tag: pin.tag.clone(),
            digest: pin.digest.clone(),
        }),
    }

    Ok(())
}

fn missing_document(dir: &str, pin: &Pinned) -> KataError {
    KataError::Document(format!("no {dir} document to retag {}", pin.identity))
}

trait NamedEntries {
    fn named_entries_mut(&mut self) -> &mut [NamedEntry];
    fn push_named(&mut self, entry: NamedEntry);
}

impl NamedEntries for OverlayDocument {
    fn named_entries_mut(&mut self) -> &mut [NamedEntry] {
        &mut self.overlays
    }

    fn push_named(&mut self, entry: NamedEntry) {
        self.overlays.push(entry);
    }
}

impl NamedEntries for ExtensionDocument {
    fn named_entries_mut(&mut self) -> &mut [NamedEntry] {
        &mut self.extensions
    }

    fn push_named(&mut self, entry: NamedEntry) {
        self.extensions.push(entry);
    }
}
