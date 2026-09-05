//! OCI manifest annotation writing.

pub(crate) mod signature;

use p256::ecdsa::SigningKey;

use crate::error::Result;
use crate::image::manifest;
use crate::pull;
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

/// Apply `mutation` to the plain manifest of an image, or to every platform manifest of an index.
async fn rewrite(reference: &str, include_root: bool, mutation: Mutation<'_>) -> Result<()> {
    let session = Session::new(reference, Access::PullPush, None).await?;
    let root_json = fetch_manifest(&session, &session.image.manifest_ref).await?;
    let parsed = manifest::parse(&root_json)?;

    for descriptor in &parsed.manifests {
        let platform_json = fetch_manifest(&session, &descriptor.digest).await?;
        mutation
            .apply(&session, &descriptor.digest, &platform_json)
            .await?;
    }

    if include_root || parsed.manifests.is_empty() {
        mutation
            .apply(&session, &session.image.manifest_ref, &root_json)
            .await?;
    }

    Ok(())
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
    /// Transform one manifest and PUT it back under the same reference.
    async fn apply(self, session: &Session, manifest_ref: &str, manifest_json: &str) -> Result<()> {
        match self {
            Mutation::Sign { key, annotation } => {
                let (body, content_type) = signature::inject(manifest_json, key, annotation)?;
                manifest::put(session, manifest_ref, &content_type, body).await
            }
            Mutation::Sizes {
                annotation,
                exclude,
            } => {
                let parsed = manifest::parse(manifest_json)?;
                let sizes = pull::layer::entry_sizes(session, &parsed.layers, exclude).await?;
                eprintln!("Annotating {} file(s)", sizes.len());
                let sizes_json = serde_json::to_string(&sizes)?;
                let (body, content_type) =
                    manifest::with_annotation(manifest_json, annotation, &sizes_json)?;

                manifest::put(session, manifest_ref, &content_type, body).await
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
