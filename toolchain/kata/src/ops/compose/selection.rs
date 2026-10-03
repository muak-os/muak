//! Selection keys and their value forms.

use alloc::string::String;

use crate::error::{KataError, Result};

/// One `KEY=VALUE` selection after parsing the value forms.
#[derive(Debug, Clone)]
pub(crate) enum Selection {
    /// `TAG`: retag the matched entry, keeping its repository.
    Retag {
        /// The selection key addressing the entry.
        key: String,
        /// The tag to pin.
        tag: String,
    },
    /// `REPOSITORY@TAG`: retag the matched entry, setting its repository.
    RetagIn {
        /// The selection key addressing the entry.
        key: String,
        /// The repository to set.
        repository: String,
        /// The tag to pin.
        tag: String,
    },
    /// `SOURCE@REPOSITORY@TAG`: retag with a source, or introduce the entry.
    Full {
        /// The selection key addressing the entry.
        key: String,
        /// The logical source of the payload.
        source: String,
        /// The repository to set.
        repository: String,
        /// The tag to pin.
        tag: String,
    },
}

impl Selection {
    /// The selection key addressing the entry.
    pub(crate) fn key(&self) -> &str {
        match *self {
            Self::Retag { ref key, .. }
            | Self::RetagIn { ref key, .. }
            | Self::Full { ref key, .. } => key,
        }
    }

    /// The tag to pin.
    pub(crate) fn tag(&self) -> &str {
        match *self {
            Self::Retag { ref tag, .. }
            | Self::RetagIn { ref tag, .. }
            | Self::Full { ref tag, .. } => tag,
        }
    }

    /// The repository to set, when the value carries one.
    pub(crate) fn repository(&self) -> Option<&str> {
        match *self {
            Self::Retag { .. } => None,
            Self::RetagIn { ref repository, .. } | Self::Full { ref repository, .. } => {
                Some(repository)
            }
        }
    }

    /// The source to set, when the value carries one.
    pub(crate) fn source(&self) -> Option<&str> {
        match *self {
            Self::Retag { .. } | Self::RetagIn { .. } => None,
            Self::Full { ref source, .. } => Some(source),
        }
    }
}

/// Whether a selection key addresses the entry identity.
pub(crate) fn selection_matches(key: &str, identity: &str) -> bool {
    if key == "kernel" {
        return identity.starts_with("kernels/");
    }

    identity == key
}

/// The last selection addressing `identity`, if any.
pub(crate) fn selection_for<'a>(
    selections: &'a [Selection],
    identity: &str,
) -> Option<&'a Selection> {
    selections
        .iter()
        .rev()
        .find(|selection| selection_matches(selection.key(), identity))
}

/// Parse the raw `KEY=VALUE` pairs into selections.
///
/// # Errors
///
/// Returns an error when a value is empty or carries more than two `@` separators.
pub(crate) fn parse_selections(pairs: &[(String, String)]) -> Result<Vec<Selection>> {
    pairs
        .iter()
        .map(|pair| parse_selection(&pair.0, &pair.1))
        .collect()
}

fn parse_selection(key: &str, value: &str) -> Result<Selection> {
    let parts: Vec<&str> = value.split('@').collect();
    if parts.iter().any(|part| part.is_empty()) || parts.len() > 3 {
        return Err(invalid(key, value));
    }

    if let [tag] = parts[..] {
        return Ok(Selection::Retag {
            key: key.to_owned(),
            tag: tag.to_owned(),
        });
    }
    if let [repository, tag] = parts[..] {
        return Ok(Selection::RetagIn {
            key: key.to_owned(),
            repository: repository.to_owned(),
            tag: tag.to_owned(),
        });
    }
    if let [source, repository, tag] = parts[..] {
        return Ok(Selection::Full {
            key: key.to_owned(),
            source: source.to_owned(),
            repository: repository.to_owned(),
            tag: tag.to_owned(),
        });
    }

    Err(invalid(key, value))
}

fn invalid(key: &str, value: &str) -> KataError {
    KataError::Document(format!(
        "selection '{key}={value}' must be TAG, REPOSITORY@TAG, or SOURCE@REPOSITORY@TAG"
    ))
}
