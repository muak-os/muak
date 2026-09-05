//! Merging per-platform manifests into a multi-arch OCI image index.

use hyper::Response;
use hyper::body::Bytes;
use hyper::http::header::CONTENT_TYPE;

use crate::arch::Arch;
use crate::digest::sha256_hex;
use crate::error::{KociError, Result};
use crate::image::manifest;
use crate::image::{OciDescriptor, Platform};
use crate::registry::auth::Access;
use crate::registry::http;
use crate::registry::session::Session;
use crate::registry::{
    DOCKER_MANIFEST_LIST_MEDIA_TYPE, DOCKER_MANIFEST_MEDIA_TYPE, OCI_IMAGE_INDEX_MEDIA_TYPE,
    OCI_MANIFEST_ACCEPT_HEADERS, OCI_MANIFEST_MEDIA_TYPE,
};
use crate::runtime;

/// One per-platform source of a merged index.
#[derive(Debug, Clone)]
pub struct Source {
    /// Platform architecture of the referenced manifest.
    pub arch: Arch,
    /// Tag or digest reference of the platform manifest.
    pub reference: String,
}

/// Parse an `ARCH=REF` source specification such as `amd64=v1-amd64`.
///
/// # Errors
///
/// Returns an error when the specification is not `ARCH=REF`, the
/// architecture is unknown, or the reference is empty.
pub fn parse_source(spec: &str) -> Result<Source> {
    let Some((arch, reference)) = spec.split_once('=') else {
        return Err(invalid_source(spec, "expected ARCH=REF"));
    };
    if reference.is_empty() {
        return Err(invalid_source(spec, "reference is empty"));
    }

    let arch = arch
        .parse::<Arch>()
        .map_err(|error| invalid_source(spec, &error))?;

    Ok(Source {
        arch,
        reference: reference.to_owned(),
    })
}

/// Merge per-platform manifests into an OCI image index in the registry.
///
/// # Errors
///
/// Returns an error when the session cannot be established, a source cannot
/// be fetched or verified, or the index cannot be pushed.
pub fn index(image: &str, tags: &[String], sources: &[Source]) -> Result<()> {
    runtime::runtime()?.block_on(merge_index(image, tags, sources))
}

async fn merge_index(image: &str, tags: &[String], sources: &[Source]) -> Result<()> {
    if tags.is_empty() {
        return Err(KociError::InvalidOciFormat(
            "no tags provided for the merged index".to_owned(),
        ));
    }
    if sources.is_empty() {
        return Err(KociError::InvalidOciFormat(
            "no sources provided for the merged index".to_owned(),
        ));
    }
    validate_platforms(sources)?;

    let session = Session::new(image, Access::PullPush, None).await?;
    let mut descriptors = Vec::with_capacity(sources.len());
    for source in sources {
        descriptors.push(resolve_descriptor(&session, source).await?);
    }

    let index = build_index(&descriptors)?;
    for tag in tags {
        manifest::put(&session, tag, OCI_IMAGE_INDEX_MEDIA_TYPE, index.clone()).await?;
        eprintln!(
            "Merged {} manifest(s) into {image}:{tag}",
            descriptors.len()
        );
    }

    Ok(())
}

/// Reject duplicate platform architectures in the source list.
fn validate_platforms(sources: &[Source]) -> Result<()> {
    for (position, source) in sources.iter().enumerate() {
        if sources
            .iter()
            .take(position)
            .any(|other| other.arch == source.arch)
        {
            return Err(KociError::InvalidOciFormat(format!(
                "duplicate source for architecture {}",
                source.arch
            )));
        }
    }
    Ok(())
}

/// Fetch one source manifest and describe it for the index.
async fn resolve_descriptor(session: &Session, source: &Source) -> Result<OciDescriptor> {
    let url = manifest::build_url(&session.image, &source.reference);
    let resp = http::get(
        &session.client,
        &url,
        session.authorization(),
        OCI_MANIFEST_ACCEPT_HEADERS,
    )
    .await?;
    let media_type = response_media_type(&resp)?;
    let body = http::collect_body(resp).await?;
    let digest = verify_digest(&source.reference, &body)?;

    Ok(OciDescriptor {
        media_type: Some(media_type),
        digest,
        size: manifest_size(body.len())?,
        platform: Some(Platform {
            architecture: Some(source.arch.as_str().to_owned()),
            os: Some("linux".to_owned()),
        }),
    })
}

/// Extract and validate the manifest media type of a response.
fn response_media_type<B>(resp: &Response<B>) -> Result<String> {
    let media_type = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or(OCI_MANIFEST_MEDIA_TYPE);

    match media_type {
        OCI_MANIFEST_MEDIA_TYPE | DOCKER_MANIFEST_MEDIA_TYPE => Ok(media_type.to_owned()),
        OCI_IMAGE_INDEX_MEDIA_TYPE | DOCKER_MANIFEST_LIST_MEDIA_TYPE => {
            Err(KociError::InvalidOciFormat(
                "source is already a multi-arch index; sources must be image manifests".to_owned(),
            ))
        }
        other => Err(KociError::InvalidOciFormat(format!(
            "unsupported manifest media type '{other}'"
        ))),
    }
}

/// Compute the manifest digest, verifying it against a `sha256:` reference.
fn verify_digest(reference: &str, body: &[u8]) -> Result<String> {
    let digest = format!("sha256:{}", sha256_hex(body));
    if reference.starts_with("sha256:") && reference != digest {
        return Err(KociError::DigestMismatch {
            resource: format!("source manifest {reference}"),
            expected: reference.to_owned(),
            actual: digest,
        });
    }

    Ok(digest)
}

/// Convert a manifest byte length into a descriptor size.
fn manifest_size(len: usize) -> Result<u64> {
    u64::try_from(len).map_err(|error| {
        KociError::InvalidOciFormat(format!("manifest size out of range: {error}"))
    })
}

/// Serialize the OCI index wrapping the given descriptors.
fn build_index(descriptors: &[OciDescriptor]) -> Result<Bytes> {
    let index = serde_json::json!({
        "schemaVersion": 2,
        "mediaType": OCI_IMAGE_INDEX_MEDIA_TYPE,
        "manifests": descriptors,
    });

    Ok(Bytes::from(serde_json::to_vec(&index)?))
}

/// Build an invalid-source error for a specification.
fn invalid_source(spec: &str, details: &str) -> KociError {
    KociError::InvalidOciFormat(format!("invalid source '{spec}': {details}"))
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    fn source(spec: &str) -> Source {
        parse_source(spec).expect("parse source")
    }

    #[test]
    fn parse_source_reads_tag_and_digest_references() {
        // ARRANGE / ACT
        let tag = source("amd64=v1-amd64");
        let digest = source("arm64=sha256:abc123");

        // ASSERT
        assert_eq!(tag.arch, Arch::Amd64);
        assert_eq!(tag.reference, "v1-amd64");
        assert_eq!(digest.arch, Arch::Arm64);
        assert_eq!(digest.reference, "sha256:abc123");
    }

    #[test]
    fn parse_source_rejects_malformed_specifications() {
        // ARRANGE / ACT / ASSERT
        let error = parse_source("v1-amd64").expect_err("missing arch should fail");
        assert!(matches!(error, KociError::InvalidOciFormat(_)));

        let error = parse_source("mips=v1").expect_err("unknown arch should fail");
        assert!(matches!(error, KociError::InvalidOciFormat(_)));

        let error = parse_source("amd64=").expect_err("empty ref should fail");
        assert!(matches!(error, KociError::InvalidOciFormat(_)));
    }

    #[test]
    fn verify_digest_accepts_tags_and_matching_digests() {
        // ARRANGE
        let body = b"manifest-bytes";

        // ACT
        let from_tag = verify_digest("v1-amd64", body).expect("verify tag source");
        let from_digest = verify_digest(&from_tag, body).expect("verify digest source");

        // ASSERT
        assert_eq!(from_tag, from_digest);
        assert!(from_tag.starts_with("sha256:"));
    }

    #[test]
    fn verify_digest_rejects_digest_references_with_wrong_content() {
        // ARRANGE
        let body = b"manifest-bytes";

        // ACT
        let error =
            verify_digest("sha256:deadbeef", body).expect_err("digest mismatch should fail");

        // ASSERT
        assert!(matches!(error, KociError::DigestMismatch { .. }));
    }

    #[test]
    fn response_media_type_accepts_image_manifests_only() {
        // ARRANGE
        let manifest = Response::builder()
            .header(CONTENT_TYPE, OCI_MANIFEST_MEDIA_TYPE)
            .body(())
            .expect("build response");
        let index = Response::builder()
            .header(CONTENT_TYPE, OCI_IMAGE_INDEX_MEDIA_TYPE)
            .body(())
            .expect("build response");
        let untyped = Response::builder().body(()).expect("build response");

        // ACT / ASSERT
        assert_eq!(
            response_media_type(&manifest).expect("image manifests are accepted"),
            OCI_MANIFEST_MEDIA_TYPE
        );
        assert!(response_media_type(&index).is_err(), "indexes are rejected");
        assert_eq!(
            response_media_type(&untyped).expect("missing content type falls back to OCI"),
            OCI_MANIFEST_MEDIA_TYPE
        );
    }

    #[test]
    fn build_index_serializes_schema_version_and_descriptors() {
        // ARRANGE
        let descriptors = vec![OciDescriptor {
            media_type: Some(OCI_MANIFEST_MEDIA_TYPE.to_owned()),
            digest: "sha256:abc".to_owned(),
            size: 123,
            platform: Some(Platform {
                architecture: Some("amd64".to_owned()),
                os: Some("linux".to_owned()),
            }),
        }];

        // ACT
        let bytes = build_index(&descriptors).expect("build index");

        // ASSERT
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).expect("parse index");
        assert_eq!(parsed.get("schemaVersion").and_then(Value::as_u64), Some(2));
        assert_eq!(
            parsed.get("mediaType").and_then(Value::as_str),
            Some("application/vnd.oci.image.index.v1+json")
        );

        let manifests = parsed
            .get("manifests")
            .and_then(Value::as_array)
            .expect("manifests array");
        let descriptor = manifests.first().expect("first descriptor");
        assert_eq!(
            descriptor.get("digest").and_then(Value::as_str),
            Some("sha256:abc")
        );
        assert_eq!(descriptor.get("size").and_then(Value::as_u64), Some(123));
        assert_eq!(
            descriptor
                .get("platform")
                .and_then(|platform| platform.get("os"))
                .and_then(Value::as_str),
            Some("linux")
        );
    }
}
