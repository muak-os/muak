//! Image config assembly.

use oci::arch::Arch;

use super::Blob;
use super::upload::blob_from_json;
#[cfg(feature = "pull")]
use crate::error::KociError;
use crate::error::Result;

/// Builds the minimal config for a scratch image with a single layer.
///
/// # Errors
///
/// Returns an error when the config JSON cannot be serialized.
pub(crate) fn scratch(arch: Arch, diff_id: &str) -> Result<Blob> {
    blob_from_json(&serde_json::json!({
        "architecture": arch.as_str(),
        "os": "linux",
        "rootfs": { "type": "layers", "diff_ids": [diff_id] },
    }))
}

/// Reads the `rootfs.diff_ids` array from an image config blob.
///
/// # Errors
///
/// Returns an error when the config is not JSON or carries no string diff ids.
#[cfg(feature = "pull")]
pub(crate) fn diff_ids(source: &[u8]) -> Result<Vec<String>> {
    let value: serde_json::Value = serde_json::from_slice(source)?;
    let entries = value
        .get("rootfs")
        .and_then(|rootfs| rootfs.get("diff_ids"))
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            KociError::PushError("source config has no rootfs.diff_ids array".to_owned())
        })?;

    entries
        .iter()
        .map(|entry| {
            entry.as_str().map(str::to_owned).ok_or_else(|| {
                KociError::PushError("source config diff_ids entry is not a string".to_owned())
            })
        })
        .collect()
}

/// Replaces `rootfs.diff_ids` in a source config, preserving every other field.
///
/// # Errors
///
/// Returns an error when the config is not JSON or has no `rootfs` object.
#[cfg(feature = "pull")]
pub(crate) fn splice(source: &[u8], diff_ids: &[String]) -> Result<Blob> {
    let mut value: serde_json::Value = serde_json::from_slice(source)?;
    let rootfs = value
        .get_mut("rootfs")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| KociError::PushError("source config has no rootfs object".to_owned()))?;
    rootfs.insert("diff_ids".to_owned(), serde_json::json!(diff_ids));

    blob_from_json(&value)
}
