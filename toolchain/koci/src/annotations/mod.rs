//! OCI manifest annotation commands.

use bytes::Bytes;
use oci::digest::sha256_hex;
use oci::error::OciError;
use oci_client::auth::Access;
use oci_client::client::Client;
use oci_client::manifest;
use p256::ecdsa::SigningKey;

use crate::error::{KociError, Result};
use crate::{registry, runtime, signature};

/// One manifest rewrite.
#[derive(Clone, Copy)]
enum Mutation<'a> {
    /// Sign the canonical manifest payload with `key`.
    Sign {
        key: &'a SigningKey,
        annotation: &'a str,
    },
}

impl Mutation<'_> {
    /// Transform one manifest, returning its new body and content type.
    fn transform(self, _client: &Client, manifest_json: &str) -> Result<(Bytes, String)> {
        match self {
            Mutation::Sign { key, annotation } => signature::inject(manifest_json, key, annotation),
        }
    }
}

/// Sign an OCI image manifest in the registry under `annotation`.
///
/// # Errors
///
/// Returns an error if the manifest cannot be fetched, signed, or pushed.
pub fn sign(reference: &str, privkey_pem: &str, annotation: &str) -> Result<()> {
    let key = signature::parse_pem_private_key(privkey_pem)?;

    runtime::runtime()?.block_on(rewrite(
        reference,
        Mutation::Sign {
            key: &key,
            annotation,
        },
    ))
}

async fn rewrite(reference: &str, mutation: Mutation<'_>) -> Result<()> {
    let client = registry::connect(reference, Access::PullPush).await?;
    let root_json = manifest::fetch(&client, &client.image().manifest_ref).await?;
    let parsed = oci::manifest::parse(&root_json)?;

    if parsed.manifests.is_empty() {
        let (body, content_type) = mutation.transform(&client, &root_json)?;

        return Ok(
            manifest::put(&client, &client.image().manifest_ref, &content_type, body).await?,
        );
    }

    let mut index: serde_json::Value = serde_json::from_str(&root_json).map_err(|error| {
        KociError::Oci(OciError::Parse(format!(
            "Failed to parse manifest JSON: {error}"
        )))
    })?;
    let entries = index
        .get_mut("manifests")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| {
            KociError::Oci(OciError::InvalidFormat(
                "Index manifests is not an array".to_owned(),
            ))
        })?;

    for (entry, descriptor) in entries.iter_mut().zip(&parsed.manifests) {
        let platform_json = manifest::fetch(&client, &descriptor.digest).await?;
        let (body, content_type) = mutation.transform(&client, &platform_json)?;
        let digest = format!("sha256:{}", sha256_hex(&body));
        manifest::put(&client, &digest, &content_type, body.clone()).await?;
        entry["digest"] = serde_json::Value::String(digest);
        entry["size"] = serde_json::Value::from(body.len());
    }

    let (body, content_type) = mutation.transform(&client, &serde_json::to_string(&index)?)?;

    manifest::put(&client, &client.image().manifest_ref, &content_type, body).await?;

    Ok(())
}
