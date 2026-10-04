//! OCI layer blob downloading, decompression, and cached-content access.

use std::fs::File;
use std::io::Read;

use flate2::read::GzDecoder;
use oci::digest::Verifier;
use oci::reference::Image;
use oci_client::blob::build_url;
use oci_client::http::{get, stream_body_to_sink};
use oci_client::transport::Transport;

use super::content::Content;
use crate::error::{KociError, Result};

/// A sequential streaming reader for one layer blob.
#[derive(Debug)]
pub enum LayerReader<'bytes> {
    /// A plain (uncompressed) layer streamed from the store.
    PlainFile(File),
    /// A gzip layer streaming from the store.
    GzippedFile(GzDecoder<File>),
    /// A plain (uncompressed) layer held in memory.
    Plain(&'bytes [u8]),
    /// A gzip layer decompressed in memory.
    Gzipped(GzDecoder<&'bytes [u8]>),
}

impl Read for LayerReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match *self {
            Self::PlainFile(ref mut file) => file.read(buf),
            Self::GzippedFile(ref mut decoder) => decoder.read(buf),
            Self::Plain(ref mut bytes) => bytes.read(buf),
            Self::Gzipped(ref mut decoder) => decoder.read(buf),
        }
    }
}

/// Wrap a committed store blob in the appropriate streaming decompressor.
///
/// # Errors
///
/// Returns an error when the layer media type is not supported.
pub(crate) fn decompress_file(
    file: File,
    media_type: Option<&str>,
) -> Result<LayerReader<'static>> {
    match media_type {
        Some(
            "application/vnd.oci.image.layer.v1.tar+gzip"
            | "application/vnd.docker.image.rootfs.diff.tar.gzip",
        ) => Ok(LayerReader::GzippedFile(GzDecoder::new(file))),
        Some(
            "application/vnd.oci.image.layer.v1.tar"
            | "application/vnd.docker.image.rootfs.diff.tar",
        )
        | None => Ok(LayerReader::PlainFile(file)),
        Some(other) => Err(KociError::UnsupportedLayerMediaType(other.to_owned())),
    }
}

/// Wrap layer bytes held in memory in the appropriate streaming decompressor.
///
/// # Errors
///
/// Returns an error when the layer media type is not supported.
pub(crate) fn decompress<'bytes>(
    bytes: &'bytes [u8],
    media_type: Option<&str>,
) -> Result<LayerReader<'bytes>> {
    match media_type {
        Some(
            "application/vnd.oci.image.layer.v1.tar+gzip"
            | "application/vnd.docker.image.rootfs.diff.tar.gzip",
        ) => Ok(LayerReader::Gzipped(GzDecoder::new(bytes))),
        Some(
            "application/vnd.oci.image.layer.v1.tar"
            | "application/vnd.docker.image.rootfs.diff.tar",
        )
        | None => Ok(LayerReader::Plain(bytes)),
        Some(other) => Err(KociError::UnsupportedLayerMediaType(other.to_owned())),
    }
}

/// Download a blob from the registry into memory, verifying its digest.
///
/// # Errors
///
/// Returns an error when the download fails or the digest does not match.
pub(crate) async fn blob(
    http: &Transport,
    image_ref: &Image,
    digest: &str,
    authorization: Option<&str>,
) -> Result<Vec<u8>> {
    let url = build_url(image_ref, digest);

    let resp = get(http, &url, authorization, &[]).await?;
    let mut digest_verifier = Verifier::new(digest)?;

    let mut bytes = Vec::new();
    stream_body_to_sink(resp, &mut bytes, &mut digest_verifier).await?;
    digest_verifier.verify()?;

    Ok(bytes)
}

/// Stream a blob into the content store, verifying its digest on commit.
///
/// # Errors
///
/// Returns an error when the download fails, the digest does not match, or
/// the store ingest fails.
pub(crate) async fn fetch_into_store(
    content: &Content,
    http: &Transport,
    image_ref: &Image,
    digest: &str,
    authorization: Option<&str>,
) -> Result<()> {
    if content.has_blob(digest).is_some() {
        return Ok(());
    }

    let url = build_url(image_ref, digest);
    let resp = get(http, &url, authorization, &[]).await?;
    let mut digest_verifier = Verifier::new(digest)?;
    let mut ingest = content.blob_writer(digest)?;

    stream_body_to_sink(resp, &mut ingest, &mut digest_verifier).await?;
    ingest.commit(digest_verifier)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;

    const PLAIN_LAYER: &str = "application/vnd.oci.image.layer.v1.tar";

    fn test_store_bytes() -> (File, Vec<u8>) {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let path = tmp.path().join("layer.tar");
        let mut file = std::fs::File::create(&path).expect("create layer file");
        file.write_all(b"tar-entry-bytes").expect("write layer");
        let bytes = b"tar-entry-bytes".to_vec();

        (
            File::options().read(true).open(&path).expect("open layer"),
            bytes,
        )
    }

    #[test]
    fn decompress_file_streams_plain_blobs() {
        // ARRANGE
        let (file, bytes) = test_store_bytes();

        // ACT
        let mut reader = decompress_file(file, Some(PLAIN_LAYER)).expect("decompress");
        let mut got = Vec::new();
        reader.read_to_end(&mut got).expect("read");

        // ASSERT
        assert_eq!(got, bytes);
    }

    #[test]
    fn decompress_file_rejects_unsupported_media_types() {
        // ARRANGE
        let (file, _) = test_store_bytes();

        // ACT / ASSERT
        let error =
            decompress_file(file, Some("application/x-something")).expect_err("unsupported");
        assert!(
            error.to_string().contains("application/x-something"),
            "the error must name the media type: {error}"
        );
    }

    #[test]
    fn decompress_streams_plain_and_gzip_bytes() {
        // ARRANGE
        let mut gzipped = Vec::new();
        let mut encoder = GzEncoder::new(&mut gzipped, Compression::default());
        encoder.write_all(b"uncompressed").expect("gzip write");
        encoder.finish().expect("gzip finish");

        // ACT / ASSERT
        let mut plain = decompress(b"uncompressed", Some(PLAIN_LAYER)).expect("decompress");
        let mut got = Vec::new();
        plain.read_to_end(&mut got).expect("read");
        assert_eq!(got, b"uncompressed");

        let mut layer = decompress(
            &gzipped,
            Some("application/vnd.oci.image.layer.v1.tar+gzip"),
        )
        .expect("decompress");
        let mut got = Vec::new();
        layer.read_to_end(&mut got).expect("read");
        assert_eq!(got, b"uncompressed");
    }
}
