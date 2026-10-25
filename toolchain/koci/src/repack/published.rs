//! Repacks a published image's layers into eStargz, in place.

use bytes::Bytes;
use oci::digest::sha256_hex;
use oci::media::{
    DOCKER_MANIFEST_LIST_MEDIA_TYPE, DOCKER_MANIFEST_MEDIA_TYPE, OCI_IMAGE_INDEX_MEDIA_TYPE,
    OCI_MANIFEST_MEDIA_TYPE,
};
use oci_client::client::Client;
use oci_client::manifest;
use serde_json::Value;

use super::platform;
use crate::error::{KociError, Result};
use crate::image::upload;
use crate::progress::{Event, Operation, Phase, Reporter, Unit};

/// Repacks every platform of the image and returns the root manifest digest.
///
/// # Errors
///
/// Returns an error when the manifest cannot be fetched, a layer cannot be
/// repacked, or a registry write fails.
pub(crate) async fn run(
    client: &Client,
    tags: &[String],
    subject: &str,
    reporter: &dyn Reporter,
) -> Result<String> {
    reporter.report(Event::Started {
        operation: Operation::Repack,
        subject,
    });
    let tags = upload::effective_tags(client, tags)?;
    let reference = client.image().manifest_ref.clone();
    let root_json = manifest::fetch(client, &reference).await?;
    let root = oci::manifest::parse(&root_json)?;
    let media = media_type(&root_json);

    let (body, digest) = if root.manifests.is_empty() {
        reporter.report(Event::IndexFetched { platforms: 1 });
        repack_manifest(client, &root_json, reporter).await?
    } else {
        reporter.report(Event::IndexFetched {
            platforms: root.manifests.len(),
        });
        repack_index(client, &root_json, reporter).await?
    };
    for tag in &tags {
        manifest::put(client, tag, media, body.clone()).await?;
        reporter.report(Event::ManifestPushed { reference: tag });
    }
    reporter.report(Event::Done {
        digest: &digest,
        tags: &tags,
    });

    Ok(digest)
}

async fn repack_index(
    client: &Client,
    root_json: &str,
    reporter: &dyn Reporter,
) -> Result<(Bytes, String)> {
    let mut index: Value = serde_json::from_str(root_json)?;
    let lines = index
        .get_mut("manifests")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| KociError::PushError("index without manifests".to_owned()))?;

    let total = lines.len();
    reporter.report(Event::PhaseStarted {
        phase: Phase::RepackPlatforms,
        label: "Repacking platforms",
        total: u64::try_from(total).unwrap_or(u64::MAX),
        unit: Unit::Items,
    });
    for (position, line) in lines.iter_mut().enumerate() {
        let source = line
            .get("digest")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| KociError::PushError("index line without digest".to_owned()))?;
        let _arch = line
            .get("platform")
            .and_then(|platform| platform.get("architecture"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        reporter.report(Event::Progress {
            phase: Phase::RepackPlatforms,
            current: u64::try_from(position.saturating_add(1)).unwrap_or(u64::MAX),
            total: u64::try_from(total).unwrap_or(u64::MAX),
        });
        let platform_json = manifest::fetch(client, &source).await?;
        let (body, digest) = repack_manifest(client, &platform_json, reporter).await?;
        manifest::put(client, &digest, OCI_MANIFEST_MEDIA_TYPE, body.clone()).await?;

        line["digest"] = Value::String(digest);
        line["size"] = Value::from(body.len());
        line["mediaType"] = Value::String(OCI_MANIFEST_MEDIA_TYPE.to_owned());
    }

    let bytes = serde_json::to_vec(&index)?;
    let digest = format!("sha256:{}", sha256_hex(&bytes));

    Ok((Bytes::from(bytes), digest))
}

async fn repack_manifest(
    client: &Client,
    manifest_json: &str,
    reporter: &dyn Reporter,
) -> Result<(Bytes, String)> {
    let manifest = oci::manifest::parse(manifest_json)?;
    let config = manifest
        .config
        .as_ref()
        .ok_or_else(|| KociError::PushError("manifest without a config descriptor".to_owned()))?;

    platform::repack(
        client,
        config,
        &manifest.layers,
        manifest.annotations.as_ref(),
        reporter,
    )
    .await
}

fn media_type(json: &str) -> &'static str {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return OCI_MANIFEST_MEDIA_TYPE;
    };

    match value.get("mediaType").and_then(Value::as_str) {
        Some(OCI_IMAGE_INDEX_MEDIA_TYPE) => OCI_IMAGE_INDEX_MEDIA_TYPE,
        Some(DOCKER_MANIFEST_LIST_MEDIA_TYPE) => DOCKER_MANIFEST_LIST_MEDIA_TYPE,
        Some(DOCKER_MANIFEST_MEDIA_TYPE) => DOCKER_MANIFEST_MEDIA_TYPE,
        _ => OCI_MANIFEST_MEDIA_TYPE,
    }
}
