use crate::build::archive::{self, TailParts};
use crate::error::Result;
use crate::source::extension::Metadata as ExtensionMetadata;

/// Tail parts produced by the tail archive build.
pub(crate) struct Tail {
    /// The prepared tail archive parts.
    pub parts: TailParts,
    /// Exact size in bytes of the tail archive.
    pub size: u64,
}

/// Builds the initramfs tail archive from extension data and profile bytes.
pub(crate) fn build(
    ext_data: &[(String, ExtensionMetadata, Vec<Vec<u8>>)],
    profile_bytes: &[u8],
) -> Result<Tail> {
    let parts = archive::prepare_tail_parts(ext_data, profile_bytes)?;
    let size = archive::tail_exact_size(&parts);
    Ok(Tail { parts, size })
}
