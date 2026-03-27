//! OCI manifest fetching, parsing, and platform selection.

use crate::error::{ImagerError, Result};
use crate::image::{ImageReference, OciDescriptor, OciManifest};
use crate::oci::OCI_MANIFEST_ACCEPT_HEADERS;
use crate::oci::http::{HttpClient, collect_body, get};

/// Return the OCI architecture string for the current host.
fn host_oci_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        other => other,
    }
}

/// Build the manifest URL for a given image reference and tag or digest.
pub(crate) fn build_url(image_ref: &ImageReference, reference: &str) -> String {
    format!(
        "{}://{}/v2/{}/manifests/{}",
        image_ref.scheme(),
        image_ref.registry,
        image_ref.name,
        reference
    )
}

/// Fetch and return the raw manifest JSON from the registry.
pub(crate) async fn fetch(
    client: &HttpClient,
    manifest_url: &str,
    token: Option<&str>,
) -> Result<String> {
    let resp = get(client, manifest_url, token, OCI_MANIFEST_ACCEPT_HEADERS).await?;
    let body = collect_body(resp).await?;
    String::from_utf8(body.to_vec())
        .map_err(|e| ImagerError::NetworkError(format!("Manifest response is not UTF-8: {}", e)))
}

/// Parse manifest JSON into an [`OciManifest`].
pub(crate) fn parse(json: &str) -> Result<OciManifest> {
    serde_json::from_str(json)
        .map_err(|e| ImagerError::OciParseError(format!("Failed to parse manifest: {}", e)))
}

/// Select the best matching platform manifest for the current host architecture.
pub(crate) fn select_platform(manifests: &[OciDescriptor]) -> Result<&OciDescriptor> {
    let target_arch = host_oci_arch();
    manifests
        .iter()
        .find(|m| {
            m.platform.as_ref().is_some_and(|p| {
                p.architecture.as_deref() == Some(target_arch) && p.os.as_deref() == Some("linux")
            })
        })
        .or_else(|| manifests.first())
        .ok_or_else(|| {
            ImagerError::InvalidOciFormat("No suitable manifest found in manifest list".to_string())
        })
}
