//! Raw-byte image copies between registries.

pub(crate) mod tree;

use alloc::collections::BTreeMap;

use bytes::Bytes;
use oci::digest::{Verifier, sha256_hex};
use oci_client::auth::Access;
use oci_client::blob;
use oci_client::client::Client;
use oci_client::http;
use oci_client::manifest;

use crate::error::{KociError, Result};
use crate::progress::sink::{Reported, Sink};
use crate::progress::{Event, Operation, Phase, Reporter, Unit};
use crate::registry;
use crate::runtime;

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
pub fn run(source: &str, destination: &str, reporter: &dyn Reporter) -> Result<String> {
    runtime::runtime()?.block_on(copy(source, destination, reporter))
}

async fn copy(source: &str, destination: &str, reporter: &dyn Reporter) -> Result<String> {
    let src = registry::connect(source, Access::Pull).await?;
    let dst = registry::connect(destination, Access::PullPush).await?;

    let subject = format!("{source} -> {destination}");
    reporter.report(Event::Started {
        operation: Operation::Copy,
        subject: &subject,
    });
    let root = manifest::fetch(&src, src.image().manifest_ref.as_str()).await?;
    let (root_digest, manifests, blobs) = collect_tree(&src, root).await?;
    reporter.report(Event::Collected {
        manifests: manifests.len(),
        blobs: blobs.len(),
    });

    let total = blobs.len();
    let total_bytes = blobs
        .values()
        .fold(0_u64, |sum, size| sum.saturating_add(*size));
    for (position, (digest, size)) in blobs.iter().enumerate() {
        reporter.report(Event::ItemQueued {
            phase: Phase::CopyBlobs,
            index: position.saturating_add(1),
            total,
            digest,
            size: *size,
        });
    }
    reporter.report(Event::PhaseStarted {
        phase: Phase::CopyBlobs,
        label: "Copying blobs",
        total: total_bytes,
        unit: Unit::Bytes,
    });
    for (position, (digest, size)) in blobs.iter().enumerate() {
        transfer_blob(
            &src,
            &dst,
            digest,
            *size,
            position.saturating_add(1),
            reporter,
        )
        .await?;
    }
    if total > 0 {
        reporter.report(Event::PhaseCompleted {
            phase: Phase::CopyBlobs,
            label: &format!("Copied {total}/{total} blobs"),
        });
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
            reporter.report(Event::ManifestPushed { reference: digest });
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

    let served = manifest::fetch(&dst, dst.image().manifest_ref.as_str()).await?;
    if served != root_manifest.bytes {
        return Err(KociError::CopyError(format!(
            "destination serves different bytes for {}; copy is not digest-faithful",
            dst.image().manifest_ref
        )));
    }
    reporter.report(Event::Verified {
        digest: &root_digest,
    });

    Ok(root_digest)
}

async fn collect_tree(
    src: &Client,
    root: String,
) -> Result<(
    String,
    BTreeMap<String, CopiedManifest>,
    BTreeMap<String, u64>,
)> {
    let mut manifests = BTreeMap::new();
    let mut blobs = BTreeMap::new();

    let root_digest = record_manifest(&mut manifests, &root)?;
    let mut pending: Vec<(String, usize)> = Vec::new();
    if tree::is_index(&tree::media_type(&root)?) {
        pending.extend(
            tree::children(&root)?
                .into_iter()
                .map(|digest| (digest, MAX_INDEX_DEPTH)),
        );
    } else {
        extend_blobs(&mut blobs, &root)?;
    }

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

        extend_blobs(&mut blobs, &bytes)?;
        if tree::is_index(&media_type) {
            let deeper = remaining.saturating_sub(1);
            pending.extend(
                tree::children(&bytes)?
                    .into_iter()
                    .map(|child| (child, deeper)),
            );
        }
    }

    Ok((root_digest, manifests, blobs))
}

fn extend_blobs(blobs: &mut BTreeMap<String, u64>, manifest: &str) -> Result<()> {
    for (digest, size) in tree::blobs(manifest)? {
        blobs.insert(digest, size);
    }

    Ok(())
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

async fn transfer_blob(
    src: &Client,
    dst: &Client,
    digest: &str,
    size: u64,
    index: usize,
    reporter: &dyn Reporter,
) -> Result<()> {
    if blob::exists(dst, digest).await? {
        reporter.report(Event::ItemSkipped {
            phase: Phase::CopyBlobs,
            digest,
        });
        reporter.report(Event::ItemDone {
            phase: Phase::CopyBlobs,
            index,
            size,
            digest,
        });
        return Ok(());
    }

    let url = blob::build_url(src.image(), digest);
    let response = http::get(src.http(), &url, src.authorization(), &[]).await?;
    let mut verifier = Verifier::new(digest)?;
    let mut bytes = Vec::new();
    let mut sink = Sink::new(
        &mut bytes,
        Reported::new(reporter, Phase::CopyBlobs, index, size),
        0,
    );
    http::stream_body_to_sink(response, &mut sink, &mut verifier).await?;
    verifier.verify()?;
    blob::upload(dst, digest, Bytes::from(bytes), &blob::report::Noop).await?;
    reporter.report(Event::ItemDone {
        phase: Phase::CopyBlobs,
        index,
        size,
        digest,
    });

    Ok(())
}
