//! Repacking published images into eStargz, in place.

mod platform;
mod published;

use oci_client::auth::Access;

use crate::error::Result;
use crate::registry;
use crate::runtime;

/// Repack a published image's layers into eStargz, in place.
///
/// # Errors
///
/// Returns an error when the image cannot be fetched, a layer cannot be
/// repacked, or any registry operation fails.
pub fn run(reference: &str, tags: &[String]) -> Result<String> {
    runtime::runtime()?.block_on(repack(reference, tags))
}

async fn repack(reference: &str, tags: &[String]) -> Result<String> {
    let client = registry::connect(reference, Access::PullPush).await?;

    published::run(&client, tags).await
}
