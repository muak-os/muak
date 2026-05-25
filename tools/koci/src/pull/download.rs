//! Layer download and ordered extraction.

use std::path::Path;

use tokio::task::{JoinSet, spawn_blocking};

use super::layer;
use crate::error::{KociError, Result};
use crate::image::{ImageReference, OciDescriptor};
use crate::registry::http::HttpClient;

/// Maximum number of concurrent layer downloads.
const MAX_CONCURRENT_DOWNLOADS: usize = 8;

type DownloadJoinSet = JoinSet<Result<(usize, Vec<u8>, Option<String>)>>;

/// Download all layers with bounded parallelism, then apply them in manifest order.
pub(super) async fn extract_layers(
    client: &HttpClient,
    image_ref: &ImageReference,
    layers: &[OciDescriptor],
    token: Option<&str>,
    dest: &Path,
) -> Result<()> {
    let token = token.map(str::to_owned);
    let mut downloaded: Vec<Option<(Vec<u8>, Option<String>)>> = vec![None; layers.len()];
    let mut join_set: DownloadJoinSet = JoinSet::new();
    let mut iter = layers.iter().enumerate();

    fill_download_slots(
        &mut join_set,
        &mut iter,
        client,
        image_ref,
        token.as_deref(),
    );

    while let Some(result) = join_set.join_next().await {
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                join_set.abort_all();
                return Err(KociError::DownloadError(format!(
                    "Layer download task panicked: {error}"
                )));
            }
        };
        let (index, bytes, media_type) = match result {
            Ok(downloaded_layer) => downloaded_layer,
            Err(error) => {
                join_set.abort_all();
                return Err(error);
            }
        };
        if let Err(error) = store_layer(&mut downloaded, index, bytes, media_type) {
            join_set.abort_all();
            return Err(error);
        }
        fill_download_slots(
            &mut join_set,
            &mut iter,
            client,
            image_ref,
            token.as_deref(),
        );
    }

    for (index, entry) in downloaded.into_iter().enumerate() {
        let Some((bytes, media_type)) = entry else {
            return Err(KociError::DownloadError(format!(
                "missing download result for layer {index}"
            )));
        };
        spawn_blocking({
            let dest = dest.to_path_buf();
            move || layer::extract_archive(&bytes, media_type.as_deref(), &dest)
        })
        .await
        .map_err(|e| KociError::LayerExtractionError(format!("layer {index}: {e}")))??;
    }

    Ok(())
}

fn store_layer(
    downloaded: &mut [Option<(Vec<u8>, Option<String>)>],
    index: usize,
    bytes: Vec<u8>,
    media_type: Option<String>,
) -> Result<()> {
    let Some(slot) = downloaded.get_mut(index) else {
        return Err(KociError::DownloadError(format!(
            "layer task returned invalid index {index}"
        )));
    };
    *slot = Some((bytes, media_type));
    Ok(())
}

fn fill_download_slots<'a>(
    join_set: &mut DownloadJoinSet,
    iter: &mut impl Iterator<Item = (usize, &'a OciDescriptor)>,
    client: &HttpClient,
    image_ref: &ImageReference,
    token: Option<&str>,
) {
    while join_set.len() < MAX_CONCURRENT_DOWNLOADS {
        let Some((index, layer_desc)) = iter.next() else {
            return;
        };
        let client = client.clone();
        let image_ref = image_ref.clone();
        let digest = layer_desc.digest.clone();
        let media_type = layer_desc.media_type.clone();
        let token = token.map(str::to_owned);
        join_set.spawn(async move {
            let bytes =
                layer::download_blob(&client, &image_ref, &digest, token.as_deref()).await?;
            Ok((index, bytes, media_type))
        });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    use tempfile::TempDir;

    use super::*;
    use crate::pull::test::{HttpResponse, TestRegistry, descriptor, layer_archive, sha256_digest};
    use crate::registry::http::build_client;

    #[test]
    fn store_downloaded_layer_rejects_invalid_index() {
        // ARRANGE
        let mut downloaded = vec![None];

        // ACT
        let error =
            store_layer(&mut downloaded, 1, Vec::new(), None).expect_err("store should fail");

        // ASSERT
        assert!(matches!(error, KociError::DownloadError(_)));
    }

    #[tokio::test]
    async fn download_and_extract_layers_reports_missing_download_result() {
        // ARRANGE
        let registry = TestRegistry::start(HashMap::new());
        let client = build_client();
        let output = TempDir::new().expect("create temp dir");
        let layers = vec![descriptor(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Some("application/vnd.oci.image.layer.v1.tar+gzip"),
        )];

        // ACT
        let error = extract_layers(
            &client,
            &registry.image_reference(),
            &layers,
            None,
            output.path(),
        )
        .await
        .expect_err("download should fail");

        // ASSERT
        assert!(matches!(error, KociError::DownloadError(_)));
    }

    #[tokio::test]
    async fn download_and_extract_layers_reports_spawn_blocking_join_errors() {
        // ARRANGE
        let client = build_client();
        let registry = TestRegistry::start(HashMap::new());
        let output = TempDir::new().expect("create temp dir");
        let layers: Vec<OciDescriptor> = Vec::new();

        // ACT
        let result = extract_layers(
            &client,
            &registry.image_reference(),
            &layers,
            None,
            output.path(),
        )
        .await;

        // ASSERT
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn download_and_extract_layers_applies_layers_in_manifest_order() {
        // ARRANGE
        let first_layer = layer_archive(&[("etc/message", b"first\n")]);
        let second_layer = layer_archive(&[("etc/.wh.message", b"")]);
        let first_digest = sha256_digest(&first_layer);
        let second_digest = sha256_digest(&second_layer);
        let registry = TestRegistry::start(HashMap::from([
            (
                ("GET".to_string(), format!("/v2/repo/blobs/{first_digest}")),
                HttpResponse::blob(first_layer),
            ),
            (
                ("GET".to_string(), format!("/v2/repo/blobs/{second_digest}")),
                HttpResponse::blob(second_layer),
            ),
        ]));
        let client = build_client();
        let output = TempDir::new().expect("create temp dir");
        let layers = vec![
            descriptor(
                &first_digest,
                Some("application/vnd.oci.image.layer.v1.tar+gzip"),
            ),
            descriptor(
                &second_digest,
                Some("application/vnd.oci.image.layer.v1.tar+gzip"),
            ),
        ];

        // ACT
        extract_layers(
            &client,
            &registry.image_reference(),
            &layers,
            None,
            output.path(),
        )
        .await
        .expect("download and extract layers");

        // ASSERT
        assert!(!output.path().join("etc/message").exists());
    }

    #[tokio::test]
    async fn download_and_extract_layers_downloads_blobs_in_parallel() {
        // ARRANGE
        let first_layer = layer_archive(&[("etc/first", b"first\n")]);
        let second_layer = layer_archive(&[("etc/second", b"second\n")]);
        let first_digest = sha256_digest(&first_layer);
        let second_digest = sha256_digest(&second_layer);
        let delay = Duration::from_millis(500);
        let registry = TestRegistry::start(HashMap::from([
            (
                ("GET".to_string(), format!("/v2/repo/blobs/{first_digest}")),
                HttpResponse::blob(first_layer).with_delay(delay),
            ),
            (
                ("GET".to_string(), format!("/v2/repo/blobs/{second_digest}")),
                HttpResponse::blob(second_layer).with_delay(delay),
            ),
        ]));
        let client = build_client();
        let output = TempDir::new().expect("create temp dir");
        let layers = vec![
            descriptor(
                &first_digest,
                Some("application/vnd.oci.image.layer.v1.tar+gzip"),
            ),
            descriptor(
                &second_digest,
                Some("application/vnd.oci.image.layer.v1.tar+gzip"),
            ),
        ];

        // ACT
        let started_at = Instant::now();
        extract_layers(
            &client,
            &registry.image_reference(),
            &layers,
            None,
            output.path(),
        )
        .await
        .expect("download and extract layers");

        // ASSERT
        assert!(started_at.elapsed() < Duration::from_millis(900));
        assert_eq!(
            std::fs::read_to_string(output.path().join("etc/first")).expect("read first file"),
            "first\n"
        );
        assert_eq!(
            std::fs::read_to_string(output.path().join("etc/second")).expect("read second file"),
            "second\n"
        );
    }

    #[tokio::test]
    async fn download_and_extract_layers_rejects_unsupported_layer_media_type() {
        // ARRANGE
        let layer = b"not used".to_vec();
        let digest = sha256_digest(&layer);
        let registry = TestRegistry::start(HashMap::from([(
            ("GET".to_string(), format!("/v2/repo/blobs/{digest}")),
            HttpResponse::blob(layer),
        )]));
        let client = build_client();
        let output = TempDir::new().expect("create temp dir");
        let layers = vec![descriptor(&digest, Some("application/test"))];

        // ACT
        let error = extract_layers(
            &client,
            &registry.image_reference(),
            &layers,
            None,
            output.path(),
        )
        .await
        .expect_err("download should fail");

        // ASSERT
        assert!(matches!(error, KociError::UnsupportedLayerMediaType(_)));
    }

    #[tokio::test]
    async fn download_and_extract_layers_allows_empty_layer_list() {
        // ARRANGE
        let registry = TestRegistry::start(HashMap::new());
        let client = build_client();
        let output = TempDir::new().expect("create temp dir");

        // ACT
        extract_layers(
            &client,
            &registry.image_reference(),
            &[],
            None,
            output.path(),
        )
        .await
        .expect("download should succeed");

        // ASSERT
        assert!(
            std::fs::read_dir(output.path())
                .expect("read output dir")
                .next()
                .is_none()
        );
    }
}
