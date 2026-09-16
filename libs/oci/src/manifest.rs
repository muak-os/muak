//! Pure OCI manifest operations.

use serde_json::Value;

use crate::error::{OciError, Result};
use crate::media::OCI_MANIFEST_MEDIA_TYPE;
use crate::model::{Descriptor, Manifest};

/// Parse manifest JSON into a [`Manifest`].
///
/// # Errors
///
/// Returns an error when the JSON does not describe a valid manifest.
pub fn parse(json: &str) -> Result<Manifest> {
    serde_json::from_str(json)
        .map_err(|error| OciError::Parse(format!("Failed to parse manifest: {error}")))
}

/// Select the matching platform manifest for the requested target architecture.
///
/// # Errors
///
/// Returns an error when no Linux descriptor matches the target architecture.
pub fn select_platform<'a>(
    manifests: &'a [Descriptor],
    target_arch: &str,
) -> Result<&'a Descriptor> {
    manifests
        .iter()
        .find(|descriptor| {
            descriptor.platform.as_ref().is_some_and(|platform| {
                platform.architecture.as_deref() == Some(target_arch)
                    && platform.os.as_deref() == Some("linux")
            })
        })
        .ok_or_else(|| {
            OciError::InvalidFormat(format!(
                "No linux/{target_arch} manifest found in manifest list"
            ))
        })
}

/// Set one manifest annotation, preserving the others, and serialize the manifest with its content type.
///
/// # Errors
///
/// Returns an error when the manifest is not a JSON object or the annotations entry is not a JSON object.
pub fn with_annotation(manifest_json: &str, key: &str, value: &str) -> Result<(Vec<u8>, String)> {
    let mut manifest_value: Value = serde_json::from_str(manifest_json)
        .map_err(|error| OciError::Parse(format!("Failed to parse manifest JSON: {error}")))?;

    manifest_value
        .as_object_mut()
        .ok_or_else(|| OciError::InvalidFormat("Manifest is not a JSON object".to_owned()))?
        .entry("annotations")
        .or_insert_with(|| Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .ok_or_else(|| {
            OciError::InvalidFormat("Manifest annotations is not a JSON object".to_owned())
        })?
        .insert(key.to_owned(), Value::String(value.to_owned()));

    let content_type = manifest_value
        .get("mediaType")
        .and_then(Value::as_str)
        .unwrap_or(OCI_MANIFEST_MEDIA_TYPE)
        .to_owned();

    let body = serde_json::to_vec(&manifest_value)?;

    Ok((body, content_type))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch;

    fn descriptor(digest: &str, architecture: Option<&str>, os: Option<&str>) -> Descriptor {
        Descriptor {
            media_type: None,
            digest: digest.to_owned(),
            size: 0,
            platform: Some(crate::model::Platform {
                architecture: architecture.map(str::to_owned),
                os: os.map(str::to_owned),
            }),
        }
    }

    #[test]
    fn parse_invalid_manifest_returns_error() {
        // ARRANGE / ACT
        let result = parse("not json");

        // ASSERT
        assert!(matches!(result, Err(OciError::Parse(_))));
    }

    #[test]
    fn parse_manifest_with_layers_and_platforms() {
        // ARRANGE
        let manifest_json = r#"{
            "schemaVersion": 2,
            "layers": [{
                "mediaType": "application/vnd.oci.image.layer.v1.tar",
                "digest": "sha256:abc"
            }],
            "manifests": [{
                "digest": "sha256:def",
                "platform": {
                    "architecture": "amd64",
                    "os": "linux"
                }
            }]
        }"#;

        // ACT
        let manifest = parse(manifest_json).expect("parse manifest");

        // ASSERT
        assert_eq!(manifest.layers.len(), 1);
        assert_eq!(
            manifest
                .layers
                .first()
                .and_then(|layer| layer.media_type.as_deref()),
            Some("application/vnd.oci.image.layer.v1.tar")
        );
        assert_eq!(manifest.manifests.len(), 1);
        assert_eq!(
            manifest
                .manifests
                .first()
                .expect("manifest should include a platform descriptor")
                .platform
                .as_ref()
                .and_then(|platform| platform.architecture.as_deref()),
            Some("amd64")
        );
    }

    #[test]
    fn select_platform_ignores_descriptor_without_platform() {
        // ARRANGE
        let manifests = vec![
            Descriptor {
                media_type: None,
                digest: "sha256:no-platform".to_owned(),
                size: 0,
                platform: None,
            },
            descriptor("sha256:match", Some("amd64"), Some("linux")),
        ];

        // ACT
        let selected = select_platform(&manifests, "amd64").expect("select matching manifest");

        // ASSERT
        assert_eq!(selected.digest, "sha256:match");
    }

    #[test]
    fn select_platform_rejects_non_linux_match() {
        // ARRANGE
        let manifests = vec![descriptor("sha256:wrong-os", Some("amd64"), Some("darwin"))];

        // ACT
        let error = select_platform(&manifests, "amd64").expect_err("selection should fail");

        // ASSERT
        assert!(matches!(error, OciError::InvalidFormat(_)));
    }

    #[test]
    fn select_platform_prefers_host_linux_match() {
        // ARRANGE
        let manifests = vec![
            descriptor(
                "sha256:wrong-os",
                Some(arch::host().as_str()),
                Some("windows"),
            ),
            descriptor("sha256:match", Some(arch::host().as_str()), Some("linux")),
            descriptor("sha256:wrong-arch", Some("arm64"), Some("linux")),
        ];

        // ACT
        let selected =
            select_platform(&manifests, arch::host().as_str()).expect("select matching manifest");

        // ASSERT
        assert_eq!(selected.digest, "sha256:match");
    }

    #[test]
    fn select_platform_errors_without_matching_target() {
        // ARRANGE
        let manifests = vec![
            descriptor("sha256:first", Some("arm64"), Some("windows")),
            descriptor("sha256:second", Some("386"), Some("linux")),
        ];

        // ACT
        let result = select_platform(&manifests, "amd64");

        // ASSERT
        assert!(matches!(result, Err(OciError::InvalidFormat(_))));
    }

    #[test]
    fn select_platform_errors_for_empty_manifest_list() {
        // ARRANGE / ACT
        let result = select_platform(&[], "amd64");

        // ASSERT
        assert!(matches!(result, Err(OciError::InvalidFormat(_))));
    }

    #[test]
    fn with_annotation_sets_key_and_preserves_others() {
        // ARRANGE
        let manifest_json = r#"{"schemaVersion":2,"annotations":{"dev.muak.sig":"AA"},"mediaType":"application/vnd.oci.image.manifest.v1+json","layers":[]}"#;

        // ACT
        let (body, content_type) =
            with_annotation(manifest_json, "dev.muak.sizes", "{}").expect("annotate manifest");

        // ASSERT
        let annotated: Value = serde_json::from_slice(&body).expect("parse annotated manifest");
        let annotations = annotated
            .get("annotations")
            .and_then(Value::as_object)
            .expect("annotated manifest must keep its annotations object");
        assert_eq!(
            annotations.get("dev.muak.sig").and_then(Value::as_str),
            Some("AA")
        );
        assert_eq!(
            annotations.get("dev.muak.sizes").and_then(Value::as_str),
            Some("{}")
        );
        assert_eq!(content_type, "application/vnd.oci.image.manifest.v1+json");
    }

    #[test]
    fn with_annotation_creates_annotations_map_and_defaults_content_type() {
        // ARRANGE
        let manifest_json = r#"{"schemaVersion":2,"layers":[]}"#;

        // ACT
        let (body, content_type) =
            with_annotation(manifest_json, "dev.muak.sizes", "{}\"").expect("annotate manifest");

        // ASSERT
        let annotated: Value = serde_json::from_slice(&body).expect("parse annotated manifest");
        assert_eq!(
            annotated
                .get("annotations")
                .and_then(|annotations| annotations.get("dev.muak.sizes"))
                .and_then(Value::as_str),
            Some("{}\"")
        );
        assert_eq!(content_type, "application/vnd.oci.image.manifest.v1+json");
    }

    #[test]
    fn with_annotation_rejects_non_object_manifest() {
        // ARRANGE / ACT
        let error =
            with_annotation("[]", "dev.muak.sizes", "{}").expect_err("annotate should fail");

        // ASSERT
        assert!(matches!(error, OciError::InvalidFormat(_)));
    }

    #[test]
    fn with_annotation_rejects_non_object_annotations() {
        // ARRANGE
        let manifest_json = r#"{"schemaVersion":2,"annotations":[],"layers":[]}"#;

        // ACT
        let error = with_annotation(manifest_json, "dev.muak.sizes", "{}")
            .expect_err("annotate should fail");

        // ASSERT
        assert!(matches!(error, OciError::InvalidFormat(_)));
    }
}
