//! OCI image manifest data types.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// OCI manifest structure for a platform-specific image manifest.
#[derive(Debug, Deserialize)]
pub struct Manifest {
    /// Blobs making up the image filesystem.
    #[serde(default)]
    pub layers: Vec<Descriptor>,
    /// Per-platform manifests when this manifest is an index.
    #[serde(default)]
    pub manifests: Vec<Descriptor>,
    /// Free-form annotations attached to the manifest.
    #[serde(default)]
    pub annotations: Option<HashMap<String, String>>,
}

/// OCI descriptor used to reference a blob.
#[derive(Debug, Deserialize, Serialize)]
pub struct Descriptor {
    /// Media type of the referenced blob.
    #[serde(rename = "mediaType")]
    pub media_type: Option<String>,
    /// `sha256:...` digest of the blob content.
    pub digest: String,
    /// Byte length of the blob.
    #[serde(default)]
    pub size: u64,
    /// Platform the blob targets, for index descriptors.
    #[serde(default)]
    pub platform: Option<Platform>,
}

/// Platform information for multi-architecture images.
#[derive(Debug, Deserialize, Default, Serialize)]
pub struct Platform {
    /// CPU architecture such as `amd64`.
    pub architecture: Option<String>,
    /// Operating system such as `linux`.
    pub os: Option<String>,
}
