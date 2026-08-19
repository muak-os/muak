//! OCI layer blob downloading and cache integration.

use std::io::Read;

use flate2::read::GzDecoder;

use super::cache::Store;
use crate::digest::StreamingDigest;
use crate::error::{KociError, Result};
use crate::image::ImageReference;
use crate::registry::http::{HttpClient, get, stream_body_to_vec};

/// A streaming reader that decompresses buffered layer data on the fly.
pub(crate) enum LayerReader<'a> {
    Plain(&'a [u8]),
    Gzipped(GzDecoder<&'a [u8]>),
}

impl Read for LayerReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match *self {
            Self::Plain(ref mut bytes) => bytes.read(buf),
            Self::Gzipped(ref mut decoder) => decoder.read(buf),
        }
    }
}

/// Download a blob from the registry into memory, verifying it's SHA-256 digest.
pub(crate) async fn blob(
    client: &HttpClient,
    image_ref: &ImageReference,
    digest: &str,
    token: Option<&str>,
) -> Result<Vec<u8>> {
    let blob_url = format!(
        "{}://{}/v2/{}/blobs/{}",
        image_ref.scheme(),
        image_ref.registry,
        image_ref.name,
        digest
    );

    let resp = get(client, &blob_url, token, &[]).await?;
    let mut digest_verifier = StreamingDigest::new(digest)?;

    let bytes = stream_body_to_vec(resp, &mut digest_verifier).await?;
    digest_verifier.verify()?;

    Ok(bytes)
}

/// Download a blob into memory, checking the local cache before the network.
pub(crate) async fn cached(
    cache: &Store,
    client: &HttpClient,
    image_ref: &ImageReference,
    digest: &str,
    token: Option<&str>,
) -> Result<Vec<u8>> {
    if let Some(bytes) = read_cached(cache, digest) {
        return Ok(bytes);
    }

    let bytes = blob(client, image_ref, digest, token).await?;
    cache.put_blob(digest, &bytes);

    Ok(bytes)
}

/// Wrap buffered layer data in the appropriate decompressor based on media type.
pub(crate) fn decompress<'a>(data: &'a [u8], media_type: Option<&str>) -> Result<LayerReader<'a>> {
    match media_type {
        Some(
            "application/vnd.oci.image.layer.v1.tar+gzip"
            | "application/vnd.docker.image.rootfs.diff.tar.gzip",
        ) => Ok(LayerReader::Gzipped(GzDecoder::new(data))),
        Some(
            "application/vnd.oci.image.layer.v1.tar"
            | "application/vnd.docker.image.rootfs.diff.tar",
        )
        | None => Ok(LayerReader::Plain(data)),
        Some(other) => Err(KociError::UnsupportedLayerMediaType(other.to_owned())),
    }
}

fn read_cached(cache: &Store, digest: &str) -> Option<Vec<u8>> {
    let path = cache.blob_path(digest)?;

    std::fs::read(path).ok()
}
