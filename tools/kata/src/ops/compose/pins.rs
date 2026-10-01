//! Resolution of selected tags to registry digests and their application.

use alloc::string::String;

use super::plan::{Auto, PlannedEntry};
use super::selection::{self, Selection};
use crate::error::Result;

/// The resolved `identity → tag + digest` pin of one planned entry.
pub(crate) struct Pinned {
    pub(crate) identity: String,
    pub(crate) source: Option<String>,
    pub(crate) repository: Option<String>,
    pub(crate) tag: String,
    pub(crate) digest: String,
}

/// Resolve the tag of every selected or auto-bumped entry to a digest.
///
/// # Errors
///
/// Returns an error when a registry resolution or auto discovery fails.
pub(crate) fn resolve_pins(
    selections: &[Selection],
    entries: &[PlannedEntry],
    resolve: &dyn Fn(&str, &str) -> Result<String>,
    auto: Option<&Auto<'_>>,
) -> Result<Vec<Pinned>> {
    let mut pins = Vec::new();

    for entry in entries {
        let chosen = selection::selection_for(selections, &entry.identity);
        let tag = match chosen.map(Selection::tag) {
            Some(tag) => tag.to_owned(),
            None => match auto {
                Some(auto) => auto(&entry.repository)?,
                None => continue,
            },
        };
        let repository = chosen
            .and_then(Selection::repository)
            .map_or_else(|| entry.repository.clone(), str::to_owned);
        let digest = resolve(&repository, &tag)?;
        pins.push(Pinned {
            identity: entry.identity.clone(),
            source: chosen.and_then(Selection::source).map(str::to_owned),
            repository: Some(repository),
            tag,
            digest,
        });
    }

    Ok(pins)
}
