//! Remote OCI registry pull orchestration.

use std::path::Path;

use crate::error::Result;
use crate::image::ImageReference;
use crate::image::manifest;
use crate::registry::auth::fetch_auth_token;
use crate::registry::http::build_client;
use crate::sign::verify;

mod download;
pub(crate) mod layer;
#[cfg(test)]
mod test;

/// Pull an OCI image and extract all layers to `dest`.
pub(crate) async fn pull_to_dir(
    reference: &str,
    arch: &str,
    dest: &Path,
    signature_key: Option<&str>,
) -> Result<()> {
    let image_ref = ImageReference::parse(reference);
    let client = build_client();
    let target_arch = arch.to_owned();

    let token = fetch_auth_token(&client, &image_ref.registry, &image_ref.name).await?;
    let manifest_url = manifest::build_url(&image_ref, &image_ref.manifest_ref);
    let manifest_json = manifest::fetch(&client, &manifest_url, token.as_deref()).await?;
    let manifest = manifest::parse(&manifest_json)?;

    let layers = if manifest.manifests.is_empty() {
        verify::check_signature(&manifest_json, signature_key)?;
        manifest.layers
    } else {
        verify::check_signature(&manifest_json, signature_key)?;

        let selected = manifest::select_platform(&manifest.manifests, &target_arch)?;
        let platform_url = manifest::build_url(&image_ref, &selected.digest);
        let platform_json = manifest::fetch(&client, &platform_url, token.as_deref()).await?;

        verify::check_signature(&platform_json, signature_key)?;

        manifest::parse(&platform_json)?.layers
    };

    download::extract_layers(&client, &image_ref, &layers, token.as_deref(), dest).await
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tempfile::TempDir;

    use super::*;
    use crate::error::KociError;
    use crate::pull::test::{HttpResponse, TestRegistry, manifest_json, sha256_digest};

    #[tokio::test]
    async fn pull_to_dir_rejects_non_utf8_manifest_response() {
        // ARRANGE
        let registry = TestRegistry::start(HashMap::from([(
            ("GET".to_string(), "/v2/repo/manifests/test".to_string()),
            HttpResponse::manifest(vec![0xff, 0xfe, 0xfd]),
        )]));
        let output = TempDir::new().expect("create temp dir");

        // ACT
        let error = pull_to_dir(
            &registry.reference("repo", "test"),
            "amd64",
            output.path(),
            None,
        )
        .await
        .expect_err("pull should fail");

        // ASSERT
        assert!(matches!(error, KociError::NetworkError(_)));
    }

    #[tokio::test]
    async fn pull_to_dir_rejects_invalid_manifest_json() {
        // ARRANGE
        let registry = TestRegistry::start(HashMap::from([(
            ("GET".to_string(), "/v2/repo/manifests/test".to_string()),
            HttpResponse::manifest(b"not json".to_vec()),
        )]));
        let output = TempDir::new().expect("create temp dir");

        // ACT
        let error = pull_to_dir(
            &registry.reference("repo", "test"),
            "amd64",
            output.path(),
            None,
        )
        .await
        .expect_err("pull should fail");

        // ASSERT
        assert!(matches!(error, KociError::OciParseError(_)));
    }

    #[tokio::test]
    async fn pull_to_dir_propagates_layer_media_type_errors() {
        // ARRANGE
        let layer = b"plain bytes".to_vec();
        let layer_digest = sha256_digest(&layer);
        let manifest = manifest_json(&layer_digest, layer.len(), "application/test");
        let registry = TestRegistry::start(HashMap::from([
            (
                ("GET".to_string(), "/v2/repo/manifests/test".to_string()),
                HttpResponse::manifest(manifest),
            ),
            (
                ("GET".to_string(), format!("/v2/repo/blobs/{layer_digest}")),
                HttpResponse::blob(layer),
            ),
        ]));
        let output = TempDir::new().expect("create temp dir");

        // ACT
        let error = pull_to_dir(
            &registry.reference("repo", "test"),
            "amd64",
            output.path(),
            None,
        )
        .await
        .expect_err("pull should fail");

        // ASSERT
        assert!(matches!(error, KociError::UnsupportedLayerMediaType(_)));
    }
}
