//! Shared OCI image assembly.

pub(crate) mod config;
pub(crate) mod layer;
pub(crate) mod manifest;
pub(crate) mod upload;

use bytes::Bytes;

/// An assembled blob with its digest and byte length.
#[derive(Debug)]
pub(crate) struct Blob {
    /// `sha256:...` digest of the blob bytes.
    pub(crate) digest: String,
    /// Byte length of the blob.
    pub(crate) size: u64,
    /// The blob bytes.
    pub(crate) bytes: Bytes,
}
