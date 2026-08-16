//! Remote OCI registry pull orchestration.

use crate::arch::Arch;
use crate::error::Result;
use crate::runtime;

pub mod cache;
pub(crate) mod download;
pub mod entries;
pub(crate) mod layer;
pub(crate) mod resolve;
pub(crate) mod scan;

/// Collect metadata about files in an OCI image without downloading file data.
///
/// # Errors
///
/// Returns an error if the image cannot be fetched, signature verification fails,
/// a layer cannot be decompressed, or the handler returns an error.
pub fn metadata<F>(
    reference: &str,
    arch: &Arch,
    pubkey_pem: Option<&str>,
    mut handler: F,
) -> Result<()>
where
    F: FnMut(entries::MetadataEntry) -> Result<()>,
{
    runtime::runtime()?.block_on(layer::process(
        reference,
        arch,
        pubkey_pem,
        |layer_idx, _entry, info, whiteout_layers| {
            scan::handle_metadata_entry(info, layer_idx, whiteout_layers, &mut handler)
        },
    ))
}

/// Stream file data from an OCI image.
///
/// # Errors
///
/// Returns an error if the image cannot be fetched, signature verification fails,
/// a layer cannot be decompressed, or the handler returns an error.
pub fn files<F>(
    reference: &str,
    arch: &Arch,
    pubkey_pem: Option<&str>,
    mut handler: F,
) -> Result<()>
where
    F: FnMut(entries::FileEntry) -> Result<()>,
{
    runtime::runtime()?.block_on(layer::process(
        reference,
        arch,
        pubkey_pem,
        |layer_idx, entry, info, whiteout_layers| {
            scan::handle_file_entry(entry, info, layer_idx, whiteout_layers, &mut handler)
        },
    ))
}
