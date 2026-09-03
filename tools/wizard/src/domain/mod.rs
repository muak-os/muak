//! Profile, release and resolution as the 3 layers of the domain resolution.

pub mod identity;
pub mod overlay;
pub mod profile;
pub mod release;
pub mod resolution;

use serde::de::Error;
use serde::{Deserialize as _, Deserializer};

use crate::error::{Result, WizardError};

/// Rejects an empty field value with a named validation error.
pub(crate) fn reject_empty(value: &str, field: &str) -> Result<()> {
    if value.is_empty() {
        return Err(WizardError::ProfileValidation(format!(
            "{field} must not be empty"
        )));
    }

    Ok(())
}

/// Rejects empty strings during deserialization.
pub(crate) fn non_empty<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let value = String::deserialize(deserializer)?;
    if value.is_empty() {
        return Err(Error::custom("must not be empty"));
    }

    Ok(value)
}

/// Rejects empty optional strings during deserialization.
pub(crate) fn non_empty_option<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    match Option::<String>::deserialize(deserializer)? {
        Some(value) if !value.is_empty() => Ok(Some(value)),
        _ => Err(Error::custom("must not be empty")),
    }
}

/// Rejects vectors containing empty strings during deserialization.
pub(crate) fn non_empty_vec<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<String>, D::Error> {
    let vec = Vec::<String>::deserialize(deserializer)?;
    if vec.iter().any(String::is_empty) {
        return Err(Error::custom("extension name must not be empty"));
    }

    Ok(vec)
}

/// Serializes a document to canonical TOML bytes for identity hashing.
pub(crate) fn canonical_toml<T: serde::Serialize>(value: &T) -> Result<Vec<u8>> {
    toml::to_string(value).map(String::into_bytes).map_err(|e| {
        WizardError::ProfileValidation(format!("failed to serialize document to TOML: {e}"))
    })
}
