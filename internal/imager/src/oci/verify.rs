//! OCI integrity and signature verification.

use base64ct::{Base64, Encoding};
use reqwest::Client;
use ring::signature;
use serde::Deserialize;

use crate::error::{ImagerError, Result};
use crate::image::ImageReference;
use crate::oci::http::build_authenticated_request;
use crate::oci::manifest;

/// Annotation key used by cosign to store the base64-encoded signature.
const COSIGN_SIGNATURE_ANNOTATION: &str = "dev.cosignproject.cosign/signature";

/// OCI media type for cosign simple-signing payloads.
const SIMPLE_SIGNING_MEDIA_TYPE: &str = "application/vnd.dev.cosign.simplesigning.v1+json";

/// Compute the SHA-256 hex digest of the given bytes.
pub(crate) fn sha256_hex(data: &[u8]) -> String {
    use ring::digest;
    let hash = digest::digest(&digest::SHA256, data);
    hex_encode(hash.as_ref())
}

/// Encode bytes as a lowercase hex string.
fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

/// Verify that the SHA-256 digest of a downloaded blob matches its expected OCI digest.
pub(crate) fn verify_blob_digest(data: &[u8], expected_digest: &str) -> Result<()> {
    let expected_hash =
        expected_digest
            .strip_prefix("sha256:")
            .ok_or_else(|| ImagerError::DigestMismatch {
                resource: "blob".to_string(),
                expected: expected_digest.to_string(),
                actual: "unsupported digest algorithm".to_string(),
            })?;

    let actual_hash = sha256_hex(data);

    if actual_hash != expected_hash {
        return Err(ImagerError::DigestMismatch {
            resource: expected_digest.to_string(),
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    Ok(())
}

/// Verify that a local file's content matches its expected OCI digest.
pub(crate) fn verify_local_digest(
    data: &[u8],
    expected_digest: &str,
    path: &std::path::Path,
) -> Result<()> {
    let expected_hash = expected_digest
        .strip_prefix("sha256:")
        .unwrap_or(expected_digest);
    let actual_hash = sha256_hex(data);

    if actual_hash != expected_hash {
        return Err(ImagerError::DigestMismatch {
            resource: path.display().to_string(),
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    Ok(())
}

/// Cosign `SimpleSigning` payload structure.
#[derive(Deserialize)]
struct SimpleSigning {
    critical: Critical,
}

#[derive(Deserialize)]
struct Critical {
    image: ImageIdentity,
    #[serde(rename = "type")]
    payload_type: String,
}

#[derive(Deserialize)]
struct ImageIdentity {
    #[serde(rename = "docker-manifest-digest")]
    docker_manifest_digest: String,
}

/// Cosign signature layer descriptor with annotations.
#[derive(Deserialize)]
struct CosignLayer {
    #[serde(default)]
    annotations: std::collections::HashMap<String, String>,
    digest: String,
}

/// Cosign signature manifest.
#[derive(Deserialize)]
struct CosignManifest {
    #[serde(default)]
    layers: Vec<CosignLayer>,
}

/// Verify the cosign signature for an OCI image manifest.
pub(crate) async fn check_cosign(
    client: &Client,
    image_ref: &ImageReference,
    manifest_json: &str,
    token: Option<&str>,
    pubkey_pem: Option<&str>,
) -> Result<()> {
    // No key supplied — skip signature verification entirely.
    let Some(pem) = pubkey_pem else {
        return Ok(());
    };

    let manifest_digest = format!("sha256:{}", sha256_hex(manifest_json.as_bytes()));
    let sig_tag = cosign_signature_tag(&manifest_digest);

    let sig_manifest_url = manifest::build_url(image_ref, &sig_tag);
    let sig_manifest_json = manifest::fetch(client, &sig_manifest_url, token)
        .await
        .map_err(|_| {
            ImagerError::SignatureVerificationFailed(format!(
                "No cosign signature found for {} (looked for tag {})",
                manifest_digest, sig_tag
            ))
        })?;

    let sig_manifest: CosignManifest = serde_json::from_str(&sig_manifest_json).map_err(|e| {
        ImagerError::SignatureVerificationFailed(format!(
            "Failed to parse cosign signature manifest: {}",
            e
        ))
    })?;

    let sig_layer = sig_manifest.layers.first().ok_or_else(|| {
        ImagerError::SignatureVerificationFailed(
            "Cosign signature manifest contains no layers".to_string(),
        )
    })?;

    let sig_b64 = sig_layer
        .annotations
        .get(COSIGN_SIGNATURE_ANNOTATION)
        .ok_or_else(|| {
            ImagerError::SignatureVerificationFailed(format!(
                "Cosign signature layer missing '{}' annotation",
                COSIGN_SIGNATURE_ANNOTATION
            ))
        })?;

    let payload_bytes = download_raw_blob(client, image_ref, &sig_layer.digest, token).await?;

    let payload: SimpleSigning = serde_json::from_slice(&payload_bytes).map_err(|e| {
        ImagerError::SignatureVerificationFailed(format!(
            "Failed to parse cosign SimpleSigning payload: {}",
            e
        ))
    })?;

    if payload.critical.payload_type != SIMPLE_SIGNING_MEDIA_TYPE {
        return Err(ImagerError::SignatureVerificationFailed(format!(
            "Unexpected payload type: {}",
            payload.critical.payload_type
        )));
    }

    if payload.critical.image.docker_manifest_digest != manifest_digest {
        return Err(ImagerError::SignatureVerificationFailed(format!(
            "Signature payload digest {} does not match manifest digest {}",
            payload.critical.image.docker_manifest_digest, manifest_digest
        )));
    }

    let sig_bytes = Base64::decode_vec(sig_b64).map_err(|e| {
        ImagerError::SignatureVerificationFailed(format!(
            "Failed to decode cosign signature from base64: {}",
            e
        ))
    })?;

    // Verify the ECDSA P-256 signature over the payload.
    let public_key = parse_pem_ec_public_key(pem)?;
    verify_ecdsa_p256(&public_key, &payload_bytes, &sig_bytes)?;

    Ok(())
}

/// Build the cosign signature tag from a manifest digest.
fn cosign_signature_tag(digest: &str) -> String {
    digest.replace(':', "-") + ".sig"
}

/// Download raw blob bytes without digest verification (for signature payloads).
async fn download_raw_blob(
    client: &Client,
    image_ref: &ImageReference,
    digest: &str,
    token: Option<&str>,
) -> Result<Vec<u8>> {
    let blob_url = format!(
        "{}://{}/v2/{}/blobs/{}",
        image_ref.scheme(),
        image_ref.registry,
        image_ref.name,
        digest
    );

    let response = build_authenticated_request(client, &blob_url, token, &[]).await?;
    response
        .bytes()
        .await
        .map_err(|e| {
            ImagerError::SignatureVerificationFailed(format!(
                "Failed to download cosign payload blob: {}",
                e
            ))
        })
        .map(|b| b.to_vec())
}

/// Parse a PEM-encoded ECDSA P-256 public key and return the raw SubjectPublicKeyInfo bytes.
fn parse_pem_ec_public_key(pem: &str) -> Result<Vec<u8>> {
    let mut b64 = String::new();
    let mut in_block = false;

    for line in pem.lines() {
        let line = line.trim();
        if line == "-----BEGIN PUBLIC KEY-----" {
            in_block = true;
            continue;
        }
        if line == "-----END PUBLIC KEY-----" {
            break;
        }
        if in_block {
            b64.push_str(line);
        }
    }

    if b64.is_empty() {
        return Err(ImagerError::SignatureVerificationFailed(
            "Failed to parse cosign public key PEM: no key data found".to_string(),
        ));
    }

    Base64::decode_vec(&b64).map_err(|e| {
        ImagerError::SignatureVerificationFailed(format!(
            "Failed to decode cosign public key from base64: {}",
            e
        ))
    })
}

/// Verify an ECDSA P-256 SHA-256 signature.
fn verify_ecdsa_p256(public_key_der: &[u8], message: &[u8], sig: &[u8]) -> Result<()> {
    let public_key =
        signature::UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_ASN1, public_key_der);

    public_key.verify(message, sig).map_err(|_| {
        ImagerError::SignatureVerificationFailed(
            "ECDSA P-256 signature verification failed: the image was not signed by the trusted key"
                .to_string(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosign_signature_tag() {
        let digest = "sha256:abcdef1234567890";
        assert_eq!(cosign_signature_tag(digest), "sha256-abcdef1234567890.sig");
    }

    #[test]
    fn test_cosign_signature_tag_no_colon() {
        let digest = "abcdef1234567890";
        assert_eq!(cosign_signature_tag(digest), "abcdef1234567890.sig");
    }

    #[test]
    fn test_parse_pem_valid() {
        let pem = "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE\n-----END PUBLIC KEY-----\n";
        let result = parse_pem_ec_public_key(pem);
        assert!(result.is_ok());
        assert!(!result.unwrap().is_empty());
    }

    #[test]
    fn test_parse_pem_empty() {
        let pem = "-----BEGIN PUBLIC KEY-----\n-----END PUBLIC KEY-----\n";
        let result = parse_pem_ec_public_key(pem);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_pem_no_markers() {
        let pem = "not a pem file";
        let result = parse_pem_ec_public_key(pem);
        assert!(result.is_err());
    }

    // ── sha256_hex ───────────────────────────────────────────────────────────

    #[test]
    fn test_sha256_hex_empty() {
        let hash = sha256_hex(b"");
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_sha256_hex_hello() {
        let hash = sha256_hex(b"hello");
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    // ── verify_blob_digest ───────────────────────────────────────────────────

    #[test]
    fn test_verify_blob_digest_ok() {
        let data = b"hello";
        let digest = "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert!(verify_blob_digest(data, digest).is_ok());
    }

    #[test]
    fn test_verify_blob_digest_mismatch() {
        let data = b"hello";
        let digest = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
        let err = verify_blob_digest(data, digest).unwrap_err();
        assert!(matches!(err, ImagerError::DigestMismatch { .. }));
    }

    #[test]
    fn test_verify_blob_digest_unsupported_algorithm() {
        let data = b"hello";
        let digest = "md5:abcdef";
        let err = verify_blob_digest(data, digest).unwrap_err();
        assert!(matches!(err, ImagerError::DigestMismatch { .. }));
    }

    // ── verify_local_digest ──────────────────────────────────────────────────

    #[test]
    fn test_verify_local_digest_ok() {
        let data = b"hello";
        let digest = "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert!(verify_local_digest(data, digest, std::path::Path::new("/fake/path")).is_ok());
    }

    #[test]
    fn test_verify_local_digest_ok_no_prefix() {
        let data = b"hello";
        let digest = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert!(verify_local_digest(data, digest, std::path::Path::new("/fake/path")).is_ok());
    }

    #[test]
    fn test_verify_local_digest_mismatch() {
        let data = b"hello";
        let digest = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
        let err =
            verify_local_digest(data, digest, std::path::Path::new("/fake/path")).unwrap_err();
        assert!(matches!(err, ImagerError::DigestMismatch { .. }));
    }
}
