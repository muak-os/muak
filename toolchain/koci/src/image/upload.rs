//! Registry upload and tag helpers.

use bytes::Bytes;
use oci::digest::sha256_hex;
use oci_client::blob;
use oci_client::client::Client;

use super::Blob;
use crate::error::{KociError, Result};

/// Converts a byte length into a descriptor size.
///
/// # Errors
///
/// Returns an error when the length does not fit a `u64`.
pub(crate) fn blob_size(len: usize) -> Result<u64> {
    u64::try_from(len)
        .map_err(|error| KociError::PushError(format!("blob size out of range: {error}")))
}

/// Wraps serialized JSON as a digest-addressed [`Blob`].
///
/// # Errors
///
/// Returns an error when the JSON cannot be serialized.
pub(crate) fn blob_from_json(value: &serde_json::Value) -> Result<Blob> {
    let bytes = Bytes::from(serde_json::to_vec(value)?);
    let digest = format!("sha256:{}", sha256_hex(&bytes));

    Ok(Blob {
        digest,
        size: blob_size(bytes.len())?,
        bytes,
    })
}

/// Uploads `bytes` unless the registry already serves `digest`.
///
/// # Errors
///
/// Returns an error when the existence check or upload fails.
pub(crate) async fn put_blob(client: &Client, digest: &str, bytes: Bytes) -> Result<()> {
    if blob::exists(client, digest).await? {
        eprintln!("Blob {digest} already in registry; skipping upload");

        return Ok(());
    }

    blob::upload(client, digest, bytes).await?;

    Ok(())
}

/// Resolves the tag list, defaulting to the reference's own tag.
///
/// # Errors
///
/// Returns an error when the reference is a digest with no explicit tag.
pub(crate) fn effective_tags(client: &Client, tags: &[String]) -> Result<Vec<String>> {
    resolve_tags(client.image().manifest_ref.as_str(), tags)
}

fn resolve_tags(reference: &str, tags: &[String]) -> Result<Vec<String>> {
    let mut resolved: Vec<String> = Vec::new();
    for tag in core::iter::once(reference).chain(tags.iter().map(String::as_str)) {
        if tag.starts_with("sha256:") || resolved.iter().any(|existing| existing == tag) {
            continue;
        }
        resolved.push(tag.to_owned());
    }
    if resolved.is_empty() {
        return Err(KociError::PushError(
            "digest reference carries no tag to push; pass --tag".to_owned(),
        ));
    }

    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_tags_appends_extra_tags_and_dedups() {
        // ARRANGE / ACT
        let tags =
            resolve_tags("v1", &["latest".to_owned(), "v1".to_owned()]).expect("resolve tags");

        // ASSERT
        assert_eq!(tags, vec!["v1".to_owned(), "latest".to_owned()]);
    }

    #[test]
    fn resolve_tags_requires_a_tag_for_digest_references() {
        // ARRANGE / ACT
        let error = resolve_tags("sha256:abc", &[]).expect_err("digest reference needs a tag");

        // ASSERT
        assert!(matches!(error, KociError::PushError(_)));
    }
}
