//! Repacks one image manifest's layers into eStargz from the registry.

use std::collections::HashMap;

use bytes::Bytes;
use oci::digest::sha256_hex;
use oci::model::Descriptor;
use oci_client::client::Client;

use crate::error::{KociError, Result};
use crate::image::{config, layer, manifest, upload};
use crate::pull::download;

/// Repacks `layers` into eStargz and returns the rebuilt manifest and digest.
///
/// # Errors
///
/// Returns an error when a blob cannot be fetched, prepared, or uploaded, or
/// the config diff ids do not match the layer count.
pub(crate) async fn repack(
    client: &Client,
    config_descriptor: &Descriptor,
    layers: &[Descriptor],
    annotations: Option<&HashMap<String, String>>,
) -> Result<(Bytes, String)> {
    let authorization = client.authorization().map(str::to_owned);
    let config_bytes = download::blob(
        client.http(),
        client.image(),
        &config_descriptor.digest,
        authorization.as_deref(),
    )
    .await?;
    let source_diff_ids = config::diff_ids(&config_bytes)?;
    if source_diff_ids.len() != layers.len() {
        return Err(KociError::PushError(format!(
            "config lists {} diff ids for {} layers",
            source_diff_ids.len(),
            layers.len()
        )));
    }

    let mut descriptors = Vec::with_capacity(layers.len());
    let mut diff_ids = Vec::with_capacity(layers.len());
    for (position, source) in layers.iter().enumerate() {
        let raw = download::blob(
            client.http(),
            client.image(),
            &source.digest,
            authorization.as_deref(),
        )
        .await?;
        let prepared = layer::prepare(raw, source)?;
        upload::put_blob(
            client,
            &prepared.descriptor.digest,
            Bytes::from(prepared.bytes),
        )
        .await?;

        let diff_id = match prepared.diff_id {
            Some(diff_id) => diff_id,
            None => source_diff_ids.get(position).cloned().ok_or_else(|| {
                KociError::PushError(format!("source config lacks diff id {position}"))
            })?,
        };
        diff_ids.push(diff_id);
        descriptors.push(prepared.descriptor);
    }

    let config = config::splice(&config_bytes, &diff_ids)?;
    upload::put_blob(client, &config.digest, config.bytes.clone()).await?;
    let bytes = manifest::build(&config, &descriptors, annotations.cloned())?;
    let digest = format!("sha256:{}", sha256_hex(&bytes));

    Ok((bytes, digest))
}
