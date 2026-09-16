//! OCI manifest annotation commands.

use bytes::Bytes;
use oci::digest::sha256_hex;
use oci::media::OCI_IMAGE_INDEX_MEDIA_TYPE;
use oci_client::auth::Access;
use oci_client::client::Client;
use oci_client::manifest;
#[cfg(feature = "sign")]
use p256::ecdsa::SigningKey;

use crate::error::{KociError, Result};
#[cfg(feature = "annotate")]
use crate::pull;
use crate::runtime;
#[cfg(feature = "sign")]
use crate::signature;

/// Sign an OCI image manifest in the registry under `annotation`.
///
/// # Errors
///
/// Returns an error if the manifest cannot be fetched, signed, or pushed.
#[cfg(feature = "sign")]
pub fn sign(reference: &str, privkey_pem: &str, annotation: &str) -> Result<()> {
    let key = signature::parse_pem_private_key(privkey_pem)?;

    runtime::runtime()?.block_on(rewrite(
        reference,
        true,
        Mutation::Sign {
            key: &key,
            annotation,
        },
    ))
}

/// Annotate an OCI image with the byte size of every file entry under `annotation`.
///
/// # Errors
///
/// Returns an error if the manifest or any layer blob cannot be fetched, or
/// the annotated manifest cannot be pushed.
#[cfg(feature = "annotate")]
pub fn sizes(reference: &str, annotation: &str, exclude: &[String]) -> Result<()> {
    runtime::runtime()?.block_on(rewrite(
        reference,
        false,
        Mutation::Sizes {
            annotation,
            exclude,
        },
    ))
}

/// Platform manifests of an index are addressed by digest: a mutated manifest
/// changes bytes, so it is pushed under its NEW digest and the index
/// descriptors are repointed before the index is pushed back under its tag.
async fn rewrite(reference: &str, include_root: bool, mutation: Mutation<'_>) -> Result<()> {
    let client = Client::new(reference, Access::PullPush, None).await?;
    let root_json = manifest::fetch(&client, &client.image().manifest_ref).await?;
    let parsed = oci::manifest::parse(&root_json)?;

    if parsed.manifests.is_empty() {
        let (body, content_type) = mutation.transform(&client, &root_json).await?;

        return Ok(
            manifest::put(&client, &client.image().manifest_ref, &content_type, body).await?,
        );
    }

    let mut index: serde_json::Value = serde_json::from_str(&root_json).map_err(|error| {
        KociError::OciParseError(format!("Failed to parse manifest JSON: {error}"))
    })?;
    let entries = index
        .get_mut("manifests")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| KociError::InvalidOciFormat("Index manifests is not an array".to_owned()))?;

    for (entry, descriptor) in entries.iter_mut().zip(&parsed.manifests) {
        let platform_json = manifest::fetch(&client, &descriptor.digest).await?;
        let (body, content_type) = mutation.transform(&client, &platform_json).await?;
        let digest = format!("sha256:{}", sha256_hex(&body));
        manifest::put(&client, &digest, &content_type, body.clone()).await?;
        entry["digest"] = serde_json::Value::String(digest);
        entry["size"] = serde_json::Value::from(body.len());
    }

    let (body, content_type) = if include_root {
        mutation
            .transform(&client, &serde_json::to_string(&index)?)
            .await?
    } else {
        let content_type = index
            .get("mediaType")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(OCI_IMAGE_INDEX_MEDIA_TYPE)
            .to_owned();
        (Bytes::from(serde_json::to_vec(&index)?), content_type)
    };

    manifest::put(&client, &client.image().manifest_ref, &content_type, body).await?;

    Ok(())
}

/// One manifest rewrite.
#[derive(Clone, Copy)]
enum Mutation<'a> {
    /// Sign the canonical manifest payload with `key`.
    #[cfg(feature = "sign")]
    Sign {
        key: &'a SigningKey,
        annotation: &'a str,
    },
    /// Measure the layer entries and store the sizes map.
    #[cfg(feature = "annotate")]
    Sizes {
        annotation: &'a str,
        exclude: &'a [String],
    },
}

impl Mutation<'_> {
    /// Transform one manifest, returning its new body and content type.
    async fn transform(self, client: &Client, manifest_json: &str) -> Result<(Bytes, String)> {
        match self {
            #[cfg(feature = "sign")]
            Mutation::Sign { key, annotation } => signature::inject(manifest_json, key, annotation),
            #[cfg(feature = "annotate")]
            Mutation::Sizes {
                annotation,
                exclude,
            } => {
                let parsed = oci::manifest::parse(manifest_json)?;
                let cache = pull::cache::Store::new();
                let sizes =
                    pull::layer::entry_sizes(client, &cache, &parsed.layers, exclude).await?;
                eprintln!("Annotating {} file(s)", sizes.len());
                let sizes_json = serde_json::to_string(&sizes)?;
                let (body, content_type) =
                    oci::manifest::with_annotation(manifest_json, annotation, &sizes_json)?;

                Ok((Bytes::from(body), content_type))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;

    #[test]
    fn serialized_sizes_map_is_a_compact_json_object() {
        // ARRANGE
        let sizes = BTreeMap::from([("vmlinuz".to_owned(), 12_345_u64)]);

        // ACT
        let json = serde_json::to_string(&sizes).expect("serialize sizes");

        // ASSERT
        assert_eq!(json, r#"{"vmlinuz":12345}"#);
    }
}
