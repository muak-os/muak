//! Selections that introduce entries no document carries yet.

use alloc::collections::BTreeMap;
use alloc::string::String;

use super::merge::Bases;
use super::plan::{Action, PlannedEntry};
use super::selection::{self, Selection};
use crate::error::{KataError, Result};
use crate::schema::documents::{CoreDocument, ExtensionDocument, OverlayDocument};
use crate::schema::kinds::{self, Kind};

/// An entry a selection introduces because no document carries it yet.
#[derive(Debug, Clone)]
pub(crate) struct Introduction {
    /// The selection identity (`kernels/SOURCE`, `overlays/NAME`, ...).
    pub(crate) identity: String,
    /// The document the entry belongs to.
    pub(crate) kind: Kind,
    /// The entry name, for overlays and extensions.
    pub(crate) name: Option<String>,
    /// The logical source of the payload.
    pub(crate) source: String,
    /// The repository to pin.
    pub(crate) repository: String,
    /// The tag to pin.
    pub(crate) tag: String,
}

impl Introduction {
    pub(crate) fn planned(&self) -> PlannedEntry {
        PlannedEntry {
            identity: self.identity.clone(),
            kind: self.kind,
            name: self.name.clone(),
            source: self.source.clone(),
            repository: self.repository.clone(),
            tag: self.tag.clone(),
            digest: String::new(),
            action: Action::Pin,
        }
    }
}

/// Classify the selections, returning the entries they introduce.
///
/// # Errors
///
/// Returns an error when a selection without a matching entry cannot be introduced.
pub(crate) fn classify(
    selections: &[Selection],
    entries: &[PlannedEntry],
) -> Result<Vec<Introduction>> {
    let mut introductions: BTreeMap<String, Introduction> = BTreeMap::new();

    for selection in selections {
        let key = selection.key();
        if entries
            .iter()
            .any(|entry| selection::selection_matches(key, &entry.identity))
        {
            continue;
        }

        let introduction = match *selection {
            Selection::Retag { .. } => {
                return Err(KataError::Document(format!(
                    "selection '{key}' has no entry to retag; introduce it with \
                     SOURCE@REPOSITORY@TAG"
                )));
            }
            Selection::RetagIn {
                ref repository,
                ref tag,
                ..
            } => match key_source(key) {
                Some(source) => introduction(key, &source, repository, tag)?,
                None => {
                    return Err(KataError::Document(format!(
                        "selection '{key}' has no entry to retag; introduce it with \
                         SOURCE@REPOSITORY@TAG"
                    )));
                }
            },
            Selection::Full {
                ref source,
                ref repository,
                ref tag,
                ..
            } => introduction(key, source, repository, tag)?,
        };
        introductions.insert(introduction.identity.clone(), introduction);
    }

    Ok(introductions.into_values().collect())
}

/// Create the documents introductions demand when no base carries them.
pub(crate) fn ensure_documents(bases: &mut Bases, introductions: &[Introduction], release: &str) {
    if introductions.iter().any(|intro| intro.kind == Kind::Core) && bases.core.is_none() {
        bases.core = Some(CoreDocument {
            api_version: kinds::CORE_API_VERSION.to_owned(),
            release: release.to_owned(),
            kernels: Vec::new(),
            stub: None,
            installer: None,
        });
    }
    if introductions
        .iter()
        .any(|intro| intro.kind == Kind::Overlays)
        && bases.overlays.is_none()
    {
        bases.overlays = Some(OverlayDocument {
            api_version: kinds::OVERLAYS_API_VERSION.to_owned(),
            release: release.to_owned(),
            overlays: Vec::new(),
        });
    }
    if introductions
        .iter()
        .any(|intro| intro.kind == Kind::Extensions)
        && bases.extensions.is_none()
    {
        bases.extensions = Some(ExtensionDocument {
            api_version: kinds::EXTENSIONS_API_VERSION.to_owned(),
            release: release.to_owned(),
            extensions: Vec::new(),
        });
    }
}

fn key_source(key: &str) -> Option<String> {
    key.strip_prefix("kernels/").map(str::to_owned)
}

fn introduction(key: &str, source: &str, repository: &str, tag: &str) -> Result<Introduction> {
    if key == "stub" || key == "installer" {
        return Ok(Introduction {
            identity: key.to_owned(),
            kind: Kind::Core,
            name: None,
            source: source.to_owned(),
            repository: repository.to_owned(),
            tag: tag.to_owned(),
        });
    }
    if key == "kernel" {
        return Err(KataError::Document(
            "selection 'kernel' cannot introduce an entry; use \
             kernels/SOURCE=REPOSITORY@TAG"
                .to_owned(),
        ));
    }

    let Some((dir, name)) = key.split_once('/') else {
        return Err(KataError::Document(format!(
            "unknown selection key '{key}'"
        )));
    };
    match dir {
        "kernels" => Ok(Introduction {
            identity: key.to_owned(),
            kind: Kind::Core,
            name: None,
            source: name.to_owned(),
            repository: repository.to_owned(),
            tag: tag.to_owned(),
        }),
        "overlays" | "extensions" => Ok(Introduction {
            identity: key.to_owned(),
            kind: if dir == "overlays" {
                Kind::Overlays
            } else {
                Kind::Extensions
            },
            name: Some(name.to_owned()),
            source: source.to_owned(),
            repository: repository.to_owned(),
            tag: tag.to_owned(),
        }),
        _ => Err(KataError::Document(format!(
            "unknown selection key '{key}'"
        ))),
    }
}
