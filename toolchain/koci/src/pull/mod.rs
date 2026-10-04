//! Remote OCI registry pull orchestration.

use alloc::collections::BTreeMap;

use oci::arch::Arch;
use oci_client::auth::Access;

use crate::error::Result;
use crate::progress::Progress;
use crate::registry;
use crate::runtime;
use crate::signature::Verification;

pub(crate) mod blobinfo;
pub mod cache;
pub(crate) mod content;
pub mod demux;
pub mod download;
pub mod entries;
pub(crate) mod fetch;
pub(crate) mod layer;
pub(crate) mod lazy;
pub(crate) mod paths;
pub(crate) mod resolve;
pub(crate) mod scan;
pub mod session;

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
        let client = registry::connect(reference, Access::Pull).await?;
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
    progress: &dyn Progress,
    handler: F,
) -> Result<()>
where
    F: FnMut(entries::FileEntry<'_>) -> Result<()>,
{
    session::open(reference, arch, verification)?.walk(progress, handler)
}
