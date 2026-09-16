//! Resolving an image reference to platform manifests and layer descriptors.

use oci::arch::Arch;
use oci::model::Descriptor;
use oci_client::manifest;

use super::Session;
use crate::annotations::Verification;
use crate::annotations::signature;
use crate::error::Result;

/// Resolve an image reference to the ordered list of layers for the target platform.
pub(crate) async fn layers(
    session: &Session,
    arch: &Arch,
    verification: Option<&Verification<'_>>,
) -> Result<Vec<Descriptor>> {
    let manifest_json = platform_manifest_json(session, arch, verification).await?;
    let manifest = oci::manifest::parse(&manifest_json)?;

    Ok(manifest.layers)
}

/// Fetch the platform manifest JSON for the target architecture.
pub(crate) async fn platform_manifest_json(
    session: &Session,
    arch: &Arch,
    verification: Option<&Verification<'_>>,
) -> Result<String> {
    let manifest_json =
        fetch_cached_manifest(session, &session.client.image().manifest_ref).await?;
    let manifest = oci::manifest::parse(&manifest_json)?;
    signature::check_signature(&manifest_json, verification)?;

    if manifest.manifests.is_empty() {
        return Ok(manifest_json);
    }

    let selected = oci::manifest::select_platform(&manifest.manifests, arch.as_str())?;
    let platform_json = fetch_cached_manifest(session, &selected.digest).await?;
    signature::check_signature(&platform_json, verification)?;

    Ok(platform_json)
}

/// Fetch a manifest, checking the local cache before hitting the network.
async fn fetch_cached_manifest(session: &Session, manifest_ref: &str) -> Result<String> {
    if let Some(cached) = session
        .cache
        .get_manifest(session.client.image(), manifest_ref)
    {
        return Ok(cached);
    }

    let json = manifest::fetch(&session.client, manifest_ref).await?;
    session
        .cache
        .put_manifest(session.client.image(), manifest_ref, &json);

    Ok(json)
}
