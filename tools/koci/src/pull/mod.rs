//! Remote OCI registry pull orchestration.

use alloc::collections::BTreeMap;

use oci::arch::Arch;
use oci_client::auth::Access;
use oci_client::client::Client;

use crate::annotations::Verification;
use crate::error::Result;
use crate::pull::cache::Store;
use crate::runtime;

pub mod cache;
pub(crate) mod download;
pub mod entries;
pub(crate) mod layer;
pub(crate) mod paths;
pub(crate) mod resolve;
pub(crate) mod scan;

/// Registry client plus local blob cache for one pull session.
pub(crate) struct Session {
    /// Local blob and tag-manifest cache.
    pub(crate) cache: Store,
    /// Authenticated registry client.
    pub(crate) client: Client,
}

impl Session {
    pub(crate) async fn new(reference: &str, access: Access) -> Result<Self> {
        Ok(Self {
            cache: Store::new(),
            client: Client::new(reference, access, None).await?,
        })
    }
}

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
        let session = Session::new(reference, Access::Pull).await?;
        let json = resolve::platform_manifest_json(&session, arch, verification).await?;
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
