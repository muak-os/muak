//! Layer download and ordered in-memory application.

use tokio::task::{JoinSet, spawn_blocking};

use super::layer;
use crate::error::{KociError, Result};
use crate::image::{ImageReference, OciDescriptor};
use crate::pulled::PulledImage;
use crate::registry::http::HttpClient;

/// Maximum number of concurrent layer downloads.
const MAX_CONCURRENT_DOWNLOADS: usize = 8;

type DownloadJoinSet = JoinSet<Result<(usize, Vec<u8>, Option<String>)>>;

/// Download all layers with bounded parallelism, then apply them in manifest order.
pub(super) async fn pull_layers(
    client: &HttpClient,
    image_ref: &ImageReference,
    layers: &[OciDescriptor],
    token: Option<&str>,
) -> Result<PulledImage> {
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

    let mut image = PulledImage::new();
    for (index, entry) in downloaded.into_iter().enumerate() {
        let Some((bytes, media_type)) = entry else {
            return Err(KociError::DownloadError(format!(
                "missing download result for layer {index}"
            )));
        };
        image =
            spawn_blocking(move || layer::extract_archive(&bytes, media_type.as_deref(), image))
                .await
                .map_err(|e| KociError::LayerExtractionError(format!("layer {index}: {e}")))??;
    }

    Ok(image)
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
    use super::*;
    use crate::image::ImageReference;
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
    async fn pull_layers_allows_empty_layer_list() {
        // ARRANGE
        let client = build_client();
        let image_reference = ImageReference::parse("127.0.0.1:9/repo:test");

        // ACT
        let image = pull_layers(&client, &image_reference, &[], None)
            .await
            .expect("download should succeed");

        // ASSERT
        assert!(image.entries().expect("entries").is_empty());
    }
}
