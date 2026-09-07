//! OCI manifest annotation writing.

pub(crate) mod signature;

use hyper::body::Bytes;
use p256::ecdsa::SigningKey;

use crate::digest::sha256_hex;
use crate::error::{KociError, Result};
use crate::image::manifest;
use crate::pull;
use crate::registry::OCI_IMAGE_INDEX_MEDIA_TYPE;
use crate::registry::auth::Access;
use crate::registry::session::Session;
use crate::runtime;

/// Signature verification requirements for pulls.
pub struct Verification<'a> {
    /// PEM-encoded ECDSA P-256 public key trusted to have signed the manifest.
    pub pubkey_pem: &'a str,
    /// Manifest annotation key carrying the base64url DER signature.
    pub sig_annotation: &'a str,
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
    let session = Session::new(reference, Access::PullPush, None).await?;
    let root_json = fetch_manifest(&session, &session.image.manifest_ref).await?;
    let parsed = manifest::parse(&root_json)?;

    if parsed.manifests.is_empty() {
        let (body, content_type) = mutation.transform(&session, &root_json).await?;

        return manifest::put(&session, &session.image.manifest_ref, &content_type, body).await;
    }

    let mut index: serde_json::Value = serde_json::from_str(&root_json).map_err(|error| {
        KociError::OciParseError(format!("Failed to parse manifest JSON: {error}"))
    })?;
    let entries = index
        .get_mut("manifests")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| KociError::InvalidOciFormat("Index manifests is not an array".to_owned()))?;

    for (entry, descriptor) in entries.iter_mut().zip(&parsed.manifests) {
        let platform_json = fetch_manifest(&session, &descriptor.digest).await?;
        let (body, content_type) = mutation.transform(&session, &platform_json).await?;
        let digest = format!("sha256:{}", sha256_hex(&body));
        manifest::put(&session, &digest, &content_type, body.clone()).await?;
        entry["digest"] = serde_json::Value::String(digest);
        entry["size"] = serde_json::Value::from(body.len());
    }

    let (body, content_type) = if include_root {
        mutation
            .transform(&session, &serde_json::to_string(&index)?)
            .await?
    } else {
        let content_type = index
            .get("mediaType")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(OCI_IMAGE_INDEX_MEDIA_TYPE)
            .to_owned();
        (Bytes::from(serde_json::to_vec(&index)?), content_type)
    };

    manifest::put(&session, &session.image.manifest_ref, &content_type, body).await
}

/// One manifest rewrite.
#[derive(Clone, Copy)]
enum Mutation<'a> {
    /// Sign the canonical manifest payload with `key`.
    Sign {
        key: &'a SigningKey,
        annotation: &'a str,
    },
    /// Measure the layer entries and store the sizes map.
    Sizes {
        annotation: &'a str,
        exclude: &'a [String],
    },
}

impl Mutation<'_> {
    /// Transform one manifest, returning its new body and content type.
    async fn transform(self, session: &Session, manifest_json: &str) -> Result<(Bytes, String)> {
        match self {
            Mutation::Sign { key, annotation } => signature::inject(manifest_json, key, annotation),
            Mutation::Sizes {
                annotation,
                exclude,
            } => {
                let parsed = manifest::parse(manifest_json)?;
                let sizes = pull::layer::entry_sizes(session, &parsed.layers, exclude).await?;
                eprintln!("Annotating {} file(s)", sizes.len());
                let sizes_json = serde_json::to_string(&sizes)?;

                manifest::with_annotation(manifest_json, annotation, &sizes_json)
            }
        }
    }
}

/// Fetch a manifest by tag or digest reference.
async fn fetch_manifest(session: &Session, manifest_ref: &str) -> Result<String> {
    let url = manifest::build_url(&session.image, manifest_ref);

    manifest::fetch(&session.client, &url, session.authorization()).await
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
