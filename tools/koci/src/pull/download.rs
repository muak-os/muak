//! OCI layer blob downloading and cache integration.

use std::fs::File;
use std::io::Read;

use flate2::read::GzDecoder;

use super::cache::Store;
use crate::digest::StreamingDigest;
use crate::error::{KociError, Result};
use crate::image::ImageReference;
use crate::registry::http::{HttpClient, get, stream_body_to_file};

/// A streaming reader that decompresses layer data on the fly.
pub(crate) enum LayerReader {
    Plain(File),
    Gzipped(GzDecoder<File>),
}

impl Read for LayerReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match *self {
            Self::Plain(ref mut file) => file.read(buf),
            Self::Gzipped(ref mut decoder) => decoder.read(buf),
        }
    }
}

/// Download a blob from the registry to a file, verifying its SHA-256 digest.
pub(crate) async fn blob(
    client: &HttpClient,
    image_ref: &ImageReference,
    digest: &str,
    token: Option<&str>,
    dest: &std::path::Path,
) -> Result<()> {
    let blob_url = format!(
        "{}://{}/v2/{}/blobs/{}",
        image_ref.scheme(),
        image_ref.registry,
        image_ref.name,
        digest
    );

    let resp = get(client, &blob_url, token, &[]).await?;
    let mut file = File::create(dest)?;
    let mut digest_verifier = StreamingDigest::new(digest)?;

    stream_body_to_file(resp, &mut file, &mut digest_verifier).await?;

    digest_verifier.verify()
}

/// Download a blob, checking the local cache before hitting the network.
pub(crate) async fn cached(
    cache: &Store,
    client: &HttpClient,
    image_ref: &ImageReference,
    digest: &str,
    token: Option<&str>,
) -> Result<File> {
    if let Some(reader) = cache.get_blob_reader(digest) {
        return Ok(reader);
    }

    let dest_path = cache.blob_path(digest).ok_or_else(|| {
        KociError::DownloadError("no cache directory configured".to_owned())
    })?;

    if let Some(parent) = dest_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| KociError::DownloadError(format!("create cache directory: {e}")))?;
    }

    blob(client, image_ref, digest, token, &dest_path).await?;

    cache.put_blob_from_file(digest, &dest_path);
    cache
        .get_blob_reader(digest)
        .ok_or_else(|| KociError::DownloadError("failed to open cached blob".to_owned()))
}

/// Wrap a file in the appropriate decompressor based on media type.
pub(crate) fn decompress(file: File, media_type: Option<&str>) -> Result<LayerReader> {
    match media_type {
        Some(
            "application/vnd.oci.image.layer.v1.tar+gzip"
            | "application/vnd.docker.image.rootfs.diff.tar.gzip",
        ) => Ok(LayerReader::Gzipped(GzDecoder::new(file))),
        Some(
            "application/vnd.oci.image.layer.v1.tar"
            | "application/vnd.docker.image.rootfs.diff.tar",
        )
        | None => Ok(LayerReader::Plain(file)),
        Some(other) => Err(KociError::UnsupportedLayerMediaType(other.to_owned())),
    }
}
