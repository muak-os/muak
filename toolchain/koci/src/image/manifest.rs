//! OCI manifest assembly.

use std::collections::HashMap;

use bytes::Bytes;
use oci::media::{OCI_CONFIG_MEDIA_TYPE, OCI_LAYER_GZIP_MEDIA_TYPE, OCI_MANIFEST_MEDIA_TYPE};
use oci::model::Descriptor;

use super::Blob;
use crate::error::Result;
use crate::stargz::LayerBlob;
use crate::stargz::toc::{TOC_DIGEST_ANNOTATION, UNCOMPRESSED_SIZE_ANNOTATION};

/// Describes a packed eStargz layer as an OCI layer descriptor.
#[must_use]
pub(crate) fn layer_descriptor(layer: &LayerBlob) -> Descriptor {
    let mut annotations = HashMap::new();
    annotations.insert(TOC_DIGEST_ANNOTATION.to_owned(), layer.toc_digest.clone());
    annotations.insert(
        UNCOMPRESSED_SIZE_ANNOTATION.to_owned(),
        layer.uncompressed_size.to_string(),
    );

    Descriptor {
        media_type: Some(OCI_LAYER_GZIP_MEDIA_TYPE.to_owned()),
        digest: layer.digest.clone(),
        size: layer.size,
        platform: None,
        annotations: Some(annotations),
    }
}

/// Serializes a manifest for `config` and `layers`.
///
/// # Errors
///
/// Returns an error when the manifest JSON cannot be serialized.
pub(crate) fn build(
    config: &Blob,
    layers: &[Descriptor],
    annotations: Option<HashMap<String, String>>,
) -> Result<Bytes> {
    let layers = layers.iter().map(layer_value).collect::<Vec<_>>();
    let mut body = serde_json::json!({
        "schemaVersion": 2,
        "mediaType": OCI_MANIFEST_MEDIA_TYPE,
        "config": {
            "mediaType": OCI_CONFIG_MEDIA_TYPE,
            "digest": config.digest,
            "size": config.size,
        },
        "layers": layers,
    });
    if let Some(annotations) = annotations
        && let Some(object) = body.as_object_mut()
    {
        object.insert("annotations".to_owned(), serde_json::json!(annotations));
    }

    Ok(Bytes::from(serde_json::to_vec(&body)?))
}

fn layer_value(descriptor: &Descriptor) -> serde_json::Value {
    let mut value = serde_json::json!({
        "mediaType": descriptor.media_type,
        "digest": descriptor.digest,
        "size": descriptor.size,
    });
    if let Some(annotations) = descriptor.annotations.as_ref()
        && let Some(object) = value.as_object_mut()
    {
        object.insert("annotations".to_owned(), serde_json::json!(annotations));
    }

    value
}
