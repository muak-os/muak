//! Resolving an image reference to platform manifests and layer descriptors.

use crate::arch::Arch;
use crate::error::Result;
use crate::image::OciDescriptor;
use crate::image::manifest;
use crate::registry::session::Session;
use crate::sign::verify;

/// Resolve an image reference to the ordered list of layers for the target platform.
pub(crate) async fn layers(
    session: &Session,
    arch: &Arch,
    pubkey_pem: Option<&str>,
) -> Result<Vec<OciDescriptor>> {
    let manifest_json = platform_manifest_json(session, arch, pubkey_pem).await?;
    let manifest = manifest::parse(&manifest_json)?;

    Ok(manifest.layers)
}

/// Fetch the platform manifest JSON for the target architecture.
pub(crate) async fn platform_manifest_json(
    session: &Session,
    arch: &Arch,
    pubkey_pem: Option<&str>,
) -> Result<String> {
    let manifest_json = fetch_cached_manifest(session, &session.image.manifest_ref).await?;
    let manifest = manifest::parse(&manifest_json)?;
    verify::check_signature(&manifest_json, pubkey_pem)?;

    if manifest.manifests.is_empty() {
        return Ok(manifest_json);
    }

    let selected = manifest::select_platform(&manifest.manifests, arch.as_str())?;
    let platform_json = fetch_cached_manifest(session, &selected.digest).await?;
    verify::check_signature(&platform_json, pubkey_pem)?;

    Ok(platform_json)
}

/// Fetch a manifest, checking the local cache before hitting the network.
async fn fetch_cached_manifest(session: &Session, manifest_ref: &str) -> Result<String> {
    let is_digest = manifest_ref.starts_with("sha256:");

    if is_digest {
        if let Some(cached) = session
            .cache
            .blob_path(manifest_ref)
            .and_then(|path| std::fs::read_to_string(&path).ok())
        {
            return Ok(cached);
        }
        let url = manifest::build_url(&session.image, manifest_ref);
        let json = manifest::fetch(&session.client, &url, session.authorization()).await?;
        session.cache.put_blob(manifest_ref, json.as_bytes());

        Ok(json)
    } else {
        if let Some(cached) =
            session
                .cache
                .get_ref(&session.image.registry, &session.image.name, manifest_ref)
        {
            return Ok(cached);
        }
        let url = manifest::build_url(&session.image, manifest_ref);
        let json = manifest::fetch(&session.client, &url, session.authorization()).await?;
        session.cache.put_ref(
            &session.image.registry,
            &session.image.name,
            manifest_ref,
            &json,
        );

        Ok(json)
    }
}
