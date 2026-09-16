//! Remote OCI registry pull orchestration.

use alloc::collections::BTreeMap;

use oci::arch::Arch;
use oci_client::auth::Access;
use oci_client::client::Client;

use crate::error::Result;
use crate::runtime;
use crate::signature::Verification;

pub mod cache;
pub(crate) mod download;
pub mod entries;
pub(crate) mod layer;
pub(crate) mod paths;
pub(crate) mod resolve;
pub(crate) mod scan;

/// Fetch the manifest annotations of the platform manifest matching `arch`.
///
/// # Errors
///
/// Returns an error if the manifest cannot be fetched or signature
/// verification fails.
pub fn annotations(
    reference: &str,
    arch: &Arch,
    verification: Option<&Verification<'_>>,
) -> Result<BTreeMap<String, String>> {
    runtime::runtime()?.block_on(async {
        let client = Client::new(reference, Access::Pull, None).await?;
        let cache = cache::Store::new();
        let json = resolve::platform_manifest_json(&client, &cache, arch, verification).await?;
        let parsed = oci::manifest::parse(&json)?;

        Ok(parsed.annotations.unwrap_or_default().into_iter().collect())
    })
}

/// Stream file data from an OCI image.
///
/// # Errors
///
/// Returns an error if the image cannot be fetched, signature verification
/// fails, a layer cannot be decompressed, or the handler returns an error.
pub fn files<F>(
    reference: &str,
    arch: &Arch,
    verification: Option<&Verification<'_>>,
    handler: F,
) -> Result<()>
where
    F: FnMut(entries::FileEntry<'_>) -> Result<()>,
{
    runtime::runtime()?.block_on(layer::files(reference, arch, verification, handler))
}
