//! Manifest tree inspection for raw-byte image copies.

use oci::media::{DOCKER_MANIFEST_LIST_MEDIA_TYPE, OCI_IMAGE_INDEX_MEDIA_TYPE};
use serde_json::Value;

use crate::error::{KociError, Result};

/// Whether a manifest media type denotes an index over child manifests.
#[must_use]
pub(crate) fn is_index(media_type: &str) -> bool {
    media_type == OCI_IMAGE_INDEX_MEDIA_TYPE || media_type == DOCKER_MANIFEST_LIST_MEDIA_TYPE
}

/// Read the media type a manifest declares for itself.
///
/// # Errors
///
/// Returns an error when the manifest is not valid JSON or declares no media
/// type.
pub(crate) fn media_type(manifest: &str) -> Result<String> {
    let value: Value = serde_json::from_str(manifest)?;
    value
        .get("mediaType")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| KociError::CopyError("manifest declares no mediaType".to_owned()))
}

/// Digests of the child manifests an index references, in declaration order.
///
/// # Errors
///
/// Returns an error when the index is not valid JSON or a child carries no
/// digest.
pub(crate) fn children(index: &str) -> Result<Vec<String>> {
    let value: Value = serde_json::from_str(index)?;
    let Some(children) = value.get("manifests").and_then(Value::as_array) else {
        return Err(KociError::CopyError(
            "index carries no manifests array".to_owned(),
        ));
    };

    children
        .iter()
        .map(|child| {
            child
                .get("digest")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| KociError::CopyError("index child carries no digest".to_owned()))
        })
        .collect()
}

/// Digests and sizes of the config and layer blobs an image manifest references.
///
/// # Errors
///
/// Returns an error when the manifest is not valid JSON or a descriptor
/// carries no digest.
pub(crate) fn blobs(manifest: &str) -> Result<Vec<(String, u64)>> {
    let value: Value = serde_json::from_str(manifest)?;
    let config = value
        .get("config")
        .ok_or_else(|| KociError::CopyError("manifest carries no config section".to_owned()))?;
    let layers = value
        .get("layers")
        .and_then(Value::as_array)
        .ok_or_else(|| KociError::CopyError("manifest carries no layers array".to_owned()))?;

    let mut blobs = vec![descriptor_blob(config, "config")?];
    let layer_blobs = layers
        .iter()
        .map(|layer| descriptor_blob(layer, "layers"))
        .collect::<Result<Vec<_>>>()?;
    blobs.extend(layer_blobs);

    Ok(blobs)
}

fn descriptor_blob(descriptor: &Value, section: &str) -> Result<(String, u64)> {
    let digest = descriptor
        .get("digest")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| KociError::CopyError(format!("{section} descriptor carries no digest")))?;
    let size = descriptor.get("size").and_then(Value::as_u64).unwrap_or(0);

    Ok((digest, size))
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = r#"{
        "mediaType": "application/vnd.oci.image.index.v1+json",
        "manifests": [
            {"digest": "sha256:aaa", "mediaType": "application/vnd.oci.image.manifest.v1+json"},
            {"digest": "sha256:bbb", "mediaType": "application/vnd.oci.image.manifest.v1+json"}
        ]
    }"#;

    const MANIFEST: &str = r#"{
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {"digest": "sha256:ccc"},
        "layers": [
            {"digest": "sha256:ddd"},
            {"digest": "sha256:eee"}
        ]
    }"#;

    #[test]
    fn is_index_recognizes_index_media_types() {
        // ARRANGE
        let index = "application/vnd.oci.image.index.v1+json";
        let list = "application/vnd.docker.distribution.manifest.list.v2+json";
        let manifest = "application/vnd.oci.image.manifest.v1+json";

        // ACT / ASSERT
        assert!(is_index(index));
        assert!(is_index(list));
        assert!(!is_index(manifest));
    }

    #[test]
    fn media_type_reads_the_declared_type() {
        // ACT / ASSERT
        assert_eq!(
            media_type(INDEX).expect("read media type"),
            "application/vnd.oci.image.index.v1+json"
        );
    }

    #[test]
    fn media_type_rejects_missing_declaration() {
        // ARRANGE
        let manifest = r#"{"schemaVersion": 2}"#;

        // ACT / ASSERT
        media_type(manifest).expect_err("missing mediaType must fail");
    }

    #[test]
    fn children_lists_child_digests_in_order() {
        // ACT / ASSERT
        assert_eq!(
            children(INDEX).expect("read children"),
            vec!["sha256:aaa".to_owned(), "sha256:bbb".to_owned()]
        );
    }

    #[test]
    fn blobs_collects_config_and_layers_with_sizes() {
        // ACT / ASSERT
        assert_eq!(
            blobs(MANIFEST).expect("read blobs"),
            vec![
                ("sha256:ccc".to_owned(), 0),
                ("sha256:ddd".to_owned(), 0),
                ("sha256:eee".to_owned(), 0)
            ]
        );
    }

    #[test]
    fn blobs_rejects_descriptors_without_digest() {
        // ARRANGE
        let manifest = r#"{"config": {}, "layers": []}"#;

        // ACT / ASSERT
        blobs(manifest).expect_err("missing descriptor digest must fail");
    }
}
