//! Raw-byte image copies between registries.

use alloc::collections::{BTreeMap, BTreeSet};

use bytes::Bytes;
use oci::digest::{Verifier, sha256_hex};
use oci_client::auth::Access;
use oci_client::client::Client;
use oci_client::{blob, http, manifest};

use crate::error::{KociError, Result};
use crate::registry;
use crate::runtime;

pub(crate) mod tree;

/// How many index levels a copy follows before refusing deeper nesting.
const MAX_INDEX_DEPTH: usize = 8;

/// One manifest of the copied tree, kept in its original bytes.
#[derive(Debug, Clone)]
struct CopiedManifest {
    media_type: String,
    bytes: String,
}

/// Copy `source` to `destination` byte-for-byte, preserving every digest.
///
/// # Errors
///
/// Returns an error when the registry sessions cannot be established, a
/// manifest or blob cannot be transferred, or the destination ends up
/// serving different bytes than the source.
pub fn run(source: &str, destination: &str) -> Result<()> {
    runtime::runtime()?.block_on(copy(source, destination))
}

async fn copy(source: &str, destination: &str) -> Result<()> {
    let src = registry::connect(source, Access::Pull).await?;
    let dst = registry::connect(destination, Access::PullPush).await?;

    let root = manifest::fetch(&src, src.image().manifest_ref.as_str()).await?;
    let (root_digest, manifests, blobs) = collect_tree(&src, root).await?;
    eprintln!(
        "Collected {} manifests and {} blobs",
        manifests.len(),
        blobs.len()
    );

    for digest in &blobs {
        transfer_blob(&src, &dst, digest).await?;
    }

    for (digest, copied) in &manifests {
        if digest != &root_digest {
            manifest::put(
                &dst,
                digest.as_str(),
                &copied.media_type,
                Bytes::from(copied.bytes.clone()),
            )
            .await?;
            eprintln!("Pushed manifest {digest}");
        }
    }

    let root_manifest = manifests.get(&root_digest).ok_or_else(|| {
        KociError::CopyError("root manifest missing from the copy tree".to_owned())
    })?;
    manifest::put(
        &dst,
        dst.image().manifest_ref.as_str(),
        &root_manifest.media_type,
        Bytes::from(root_manifest.bytes.clone()),
    )
    .await?;
    eprintln!("Pushed manifest {root_digest}");

    let served = manifest::fetch(&dst, dst.image().manifest_ref.as_str()).await?;
    if served != root_manifest.bytes {
        return Err(KociError::CopyError(format!(
            "destination serves different bytes for {}; copy is not digest-faithful",
            dst.image().manifest_ref
        )));
    }

    eprintln!("Copied {source} to {destination} ({root_digest})");

    Ok(())
}

async fn collect_tree(
    src: &Client,
    root: String,
) -> Result<(String, BTreeMap<String, CopiedManifest>, BTreeSet<String>)> {
    let mut manifests = BTreeMap::new();
    let mut blobs = BTreeSet::new();

    let root_digest = record_manifest(&mut manifests, &root)?;
    let mut pending: Vec<(String, usize)> = tree::children(&root)?
        .into_iter()
        .map(|digest| (digest, MAX_INDEX_DEPTH))
        .collect();

    while let Some((digest, remaining)) = pending.pop() {
        if remaining == 0 {
            return Err(KociError::CopyError(
                "index nesting exceeds the supported depth".to_owned(),
            ));
        }
        if manifests.contains_key(&digest) {
            continue;
        }

        let bytes = manifest::fetch(src, &digest).await?;
        let media_type = tree::media_type(&bytes)?;
        manifests.insert(
            digest.clone(),
            CopiedManifest {
                media_type: media_type.clone(),
                bytes: bytes.clone(),
            },
        );

        if tree::is_index(&media_type) {
            blobs.extend(tree::blob_digests(&bytes)?);
            let deeper = remaining.saturating_sub(1);
            pending.extend(
                tree::children(&bytes)?
                    .into_iter()
                    .map(|child| (child, deeper)),
            );
        } else {
            blobs.extend(tree::blob_digests(&bytes)?);
        }
    }

    Ok((root_digest, manifests, blobs))
}

fn record_manifest(
    manifests: &mut BTreeMap<String, CopiedManifest>,
    bytes: &str,
) -> Result<String> {
    let media_type = tree::media_type(bytes)?;
    let digest = format!("sha256:{}", sha256_hex(bytes.as_bytes()));
    manifests.insert(
        digest.clone(),
        CopiedManifest {
            media_type,
            bytes: bytes.to_owned(),
        },
    );

    Ok(digest)
}

async fn transfer_blob(src: &Client, dst: &Client, digest: &str) -> Result<()> {
    if blob::exists(dst, digest).await? {
        eprintln!("Blob {digest} already in destination; skipping upload");

        return Ok(());
    }

    let url = blob::build_url(src.image(), digest);
    let response = http::get(src.http(), &url, src.authorization(), &[]).await?;
    let mut verifier = Verifier::new(digest)?;
    let bytes = http::stream_body_to_vec(response, &mut verifier).await?;
    verifier.verify()?;
    blob::upload(dst, digest, Bytes::from(bytes)).await?;
    eprintln!("Transferred blob {digest}");

    Ok(())
}
