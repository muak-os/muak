//! Lazy layer access downloading blobs on first read.

use oci::model::Descriptor;

use super::download::{self, LayerReader};
use super::fetch;
use super::session::Session;
use crate::error::{KociError, Result};
use crate::runtime;

/// Lazy access to an opened session's layers.
pub struct LazyLayers<'session> {
    session: &'session Session,
}

/// One layer of a session, downloaded only when its blob is read.
pub struct LazyLayer<'session> {
    session: &'session Session,
    index: usize,
    descriptor: &'session Descriptor,
}

impl<'session> LazyLayers<'session> {
    pub(crate) fn new(session: &'session Session) -> Self {
        Self { session }
    }

    /// The number of layers in the image.
    #[must_use]
    pub fn count(&self) -> usize {
        self.session.layers().len()
    }

    /// A lazy handle to the layer at `index`, in manifest order.
    ///
    /// # Errors
    ///
    /// Returns an error when `index` is out of bounds.
    pub fn layer(&self, index: usize) -> Result<LazyLayer<'_>> {
        let descriptor = self.session.layers().get(index).ok_or_else(|| {
            KociError::Pull(format!(
                "layer index {index} out of bounds ({} layer(s))",
                self.count()
            ))
        })?;

        Ok(LazyLayer {
            session: self.session,
            index,
            descriptor,
        })
    }
}

impl LazyLayer<'_> {
    /// The position of the layer in the manifest, for progress reporting.
    #[must_use]
    pub fn index(&self) -> usize {
        self.index
    }

    /// The layer descriptor (digest, size, media type).
    #[must_use]
    pub fn descriptor(&self) -> &Descriptor {
        self.descriptor
    }

    /// Open the layer blob, downloading it from the registry on first use.
    ///
    /// # Errors
    ///
    /// Returns an error when the download fails, the digest does not match, or
    /// the layer media type is not supported.
    pub fn blob_into<'bytes>(&self, buffer: &'bytes mut Vec<u8>) -> Result<LayerReader<'bytes>> {
        let session = self.session;
        let digest = &self.descriptor.digest;
        let media_type = self.descriptor.media_type.as_deref();

        let bytes = runtime::runtime()?.block_on(fetch::fetch_one(
            session.cache(),
            session.content(),
            session.client().http(),
            session.client().image(),
            digest,
            session.client().authorization(),
        ))?;

        if let Some(blob) = bytes {
            *buffer = blob;
            download::decompress(download::BlobSource::Borrowed(buffer), media_type)
        } else {
            let file = session.content().open_blob(digest)?;
            download::decompress(download::BlobSource::File(file), media_type)
        }
    }
}
