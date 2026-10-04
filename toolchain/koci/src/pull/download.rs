//! OCI layer blob downloading, decompression, and cached-content access.

use std::fs::File;
use std::io::Read;

use flate2::read::GzDecoder;
use oci::digest::Verifier;
use oci::reference::Image;
use oci_client::blob::build_url;
use oci_client::error::ClientError;
use oci_client::http::{Range, get, get_range, is_partial, stream_body_to_sink};
use oci_client::transport::Transport;

use super::content::{Content, Ingest};
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
) -> Result<u64> {
    if let Some(size) = content.has_blob(digest) {
        return Ok(size);
    }

    let mut ingest = content.blob_writer(digest)?;
    let url = build_url(image_ref, digest);
    let verifier = stream_blob(&mut ingest, http, &url, digest, authorization).await?;

    ingest.commit(verifier)
}

/// HTTP status code for a staging prefix that overshoots the blob.
const RANGE_NOT_SATISFIABLE: u16 = 416;

/// Stream the blob body into `ingest`, resuming staged bytes via Range.
async fn stream_blob(
    ingest: &mut Ingest,
    http: &Transport,
    url: &str,
    digest: &str,
    authorization: Option<&str>,
) -> Result<Verifier> {
    let offset = ingest.resume_offset();
    if offset > 0 {
        let range = Range::from_start(offset);
        return match get_range(http, url, authorization, &[], &range).await {
            Ok(resp) if is_partial(&resp) => {
                let mut digest_verifier = Verifier::new(digest)?;
                ingest.hash_prefix(&mut digest_verifier)?;
                stream_body_to_sink(resp, ingest, &mut digest_verifier).await?;

                Ok(digest_verifier)
            }
            Ok(resp) => {
                // The registry ignored the Range header, we fetch the whole blob.
                ingest.restart()?;
                let mut digest_verifier = Verifier::new(digest)?;
                stream_body_to_sink(resp, ingest, &mut digest_verifier).await?;

                Ok(digest_verifier)
            }
            Err(ClientError::Status {
                status: RANGE_NOT_SATISFIABLE,
                ..
            }) => {
                // The staged prefix overshoots the blob, we start over.
                ingest.restart()?;
                let resp = get(http, url, authorization, &[]).await?;
                let mut digest_verifier = Verifier::new(digest)?;
                stream_body_to_sink(resp, ingest, &mut digest_verifier).await?;

                Ok(digest_verifier)
            }
            Err(error) => Err(KociError::Client(error)),
        };
    }
    let resp = get(http, url, authorization, &[]).await?;
    let mut digest_verifier = Verifier::new(digest)?;
    stream_body_to_sink(resp, ingest, &mut digest_verifier).await?;

    Ok(digest_verifier)
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
