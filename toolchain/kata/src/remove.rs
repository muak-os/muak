//! Removing entries from catalog documents.

use std::path::PathBuf;

use crate::error::{KataError, Result};
use crate::progress::{Event, Reporter};
use crate::repository;
use crate::schema::documents;
use crate::schema::documents::Document;
use crate::schema::entries::{NamedEntry, SourcedEntry};
use crate::schema::kinds::Kind;
use crate::schema::parse::validate_release;
use crate::schema::view::Role;

/// One removed catalog entry.
#[derive(Debug)]
pub struct Removed {
    /// Entry identity, such as `kernels/linux`.
    pub identity: String,
    /// Digest the removed entry pinned.
    pub digest: String,
}

/// One `kata remove` request.
#[derive(Debug, Clone)]
pub struct Input {
    /// Docs root.
    pub dir: PathBuf,
    /// Entry role: kernel, stub, installer, overlays, or extensions.
    pub role: Role,
    /// Name of the named entry (overlays and extensions).
    pub name: Option<String>,
    /// Source of the kernel entry.
    pub source: Option<String>,
    /// Release line.
    pub release: String,
}

/// Remove an entry from a release line's documents.
///
/// # Errors
///
/// Returns an error when the release is invalid, the role is missing
/// `--source`/`--name`, the document is missing, the entry does not exist,
/// or the removal would leave the line unresolvable.
pub fn run(input: &Input, reporter: &dyn Reporter) -> Result<Removed> {
    validate_release(&input.release)?;

    let mut document = load(input)?;
    let removed = match input.role {
        Role::Kernel => remove_kernel(&mut document, source(input)?),
        Role::Overlay => remove_named(&mut document, Kind::Overlays, name(input)?),
        Role::Extension => remove_named(&mut document, Kind::Extensions, name(input)?),
        Role::Stub => remove_slot(&mut document, "stub"),
        Role::Installer => remove_slot(&mut document, "installer"),
    }?;

    if let Document::Core(ref core) = document {
        documents::ensure_resolvable(core)?;
    }

    repository::write(&document, &input.dir, &input.release)?;
    reporter.report(Event::Removed {
        identity: &removed.identity,
        digest: &removed.digest,
    });

    Ok(removed)
}

fn document_kind(role: Role) -> Kind {
    match role {
        Role::Kernel | Role::Stub | Role::Installer => Kind::Core,
        Role::Overlay => Kind::Overlays,
        Role::Extension => Kind::Extensions,
    }
}

fn load(input: &Input) -> Result<Document> {
    let kind = document_kind(input.role);
    if !repository::document_path(kind, &input.dir, &input.release).exists() {
        return Err(KataError::Document(format!(
            "no {} document for {}",
            kind.dir(),
            input.release
        )));
    }

    repository::load(kind, &input.dir, &input.release)
}

fn source(input: &Input) -> Result<&str> {
    input
        .source
        .as_deref()
        .ok_or_else(|| KataError::Document("removing a kernels entry requires --source".to_owned()))
}

fn name(input: &Input) -> Result<&str> {
    input.name.as_deref().ok_or_else(|| {
        KataError::Document(format!(
            "removing a {} entry requires --name",
            document_kind(input.role).dir()
        ))
    })
}

fn remove_kernel(document: &mut Document, source: &str) -> Result<Removed> {
    match *document {
        Document::Core(ref mut core) => {
            let entry = take_kernel(&mut core.kernels, source)?;

            Ok(Removed {
                identity: format!("kernels/{source}"),
                digest: entry.digest,
            })
        }
        Document::Overlays(_) | Document::Extensions(_) => Err(KataError::Document(
            "kernels entries live in a core document".to_owned(),
        )),
    }
}

fn remove_slot(document: &mut Document, slot: &str) -> Result<Removed> {
    match *document {
        Document::Core(ref mut core) => {
            let slot_entry = match slot {
                "stub" => &mut core.stub,
                _ => &mut core.installer,
            };
            let Some(entry) = slot_entry.take() else {
                return Err(KataError::Document(format!("no {slot} entry to remove")));
            };

            Ok(Removed {
                identity: slot.to_owned(),
                digest: entry.digest,
            })
        }
        Document::Overlays(_) | Document::Extensions(_) => Err(KataError::Document(
            "core slots live in a core document".to_owned(),
        )),
    }
}

fn remove_named(document: &mut Document, kind: Kind, name: &str) -> Result<Removed> {
    match *document {
        Document::Overlays(ref mut overlays) => {
            let entry = take_named(&mut overlays.overlays, name, kind)?;

            Ok(Removed {
                identity: format!("overlays/{name}"),
                digest: entry.digest,
            })
        }
        Document::Extensions(ref mut extensions) => {
            let entry = take_named(&mut extensions.extensions, name, kind)?;

            Ok(Removed {
                identity: format!("extensions/{name}"),
                digest: entry.digest,
            })
        }
        Document::Core(_) => Err(KataError::Document(format!(
            "{}.toml is not a {} document",
            kind.dir(),
            kind.dir()
        ))),
    }
}

fn take_kernel(entries: &mut Vec<SourcedEntry>, source: &str) -> Result<SourcedEntry> {
    let Some(position) = entries.iter().position(|entry| entry.source == source) else {
        return Err(KataError::Document(format!(
            "no kernels entry for source '{source}'"
        )));
    };

    Ok(entries.remove(position))
}

fn take_named(entries: &mut Vec<NamedEntry>, name: &str, kind: Kind) -> Result<NamedEntry> {
    let Some(position) = entries.iter().position(|entry| entry.name == name) else {
        return Err(KataError::Document(format!(
            "no {} entry named '{name}'",
            kind.dir()
        )));
    };

    Ok(entries.remove(position))
}
