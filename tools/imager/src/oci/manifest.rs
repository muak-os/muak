use reqwest::Client;

use crate::error::{ImagerError, Result};
use crate::image::{ImageReference, OciDescriptor, OciManifest};
use crate::oci::OCI_MANIFEST_ACCEPT_HEADERS;
use crate::oci::http::build_authenticated_request;

/// Returns the OCI architecture string for the current host.
fn host_oci_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        other => other,
    }
}

/// Build the manifest URL for an image reference and reference (tag or digest).
pub(crate) fn build_url(image_ref: &ImageReference, reference: &str) -> String {
    format!(
        "{}://{}/v2/{}/manifests/{}",
        image_ref.scheme(),
        image_ref.registry,
        image_ref.name,
        reference
    )
}

/// Fetch manifest JSON from the registry.
pub(crate) async fn fetch(
    client: &Client,
    manifest_url: &str,
    token: Option<&str>,
) -> Result<String> {
    let response =
        build_authenticated_request(client, manifest_url, token, OCI_MANIFEST_ACCEPT_HEADERS)
            .await?;
    response
        .text()
        .await
        .map_err(|e| ImagerError::NetworkError(format!("Failed to read manifest response: {}", e)))
}

/// Parse manifest JSON into OciManifest.
pub(crate) fn parse(json: &str) -> Result<OciManifest> {
    serde_json::from_str(json)
        .map_err(|e| ImagerError::OciParseError(format!("Failed to parse manifest: {}", e)))
}

/// Select the appropriate platform manifest from a manifest list.
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
