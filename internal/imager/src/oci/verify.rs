//! OCI integrity and signature verification.

use base64ct::{Base64Url, Encoding};
use ring::signature;
use serde_json::Value;

use crate::error::{ImagerError, Result};
use crate::oci::sign::extract_config_digest;

/// Annotation key used to store the image signature.
pub(crate) const SIG_ANNOTATION: &str = "dev.muak.sig";

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

/// Check the `SIG_ANNOTATION` annotation on the manifest against the provided public key.
pub(crate) async fn check_signature(manifest_json: &str, pubkey_pem: Option<&str>) -> Result<()> {
    let Some(pem) = pubkey_pem else {
        return Ok(());
    };

    let manifest_value: Value = serde_json::from_str(manifest_json).map_err(|e| {
        ImagerError::SignatureVerificationFailed(format!("Failed to parse manifest JSON: {}", e))
    })?;

    let sig_b64 = manifest_value
        .get("annotations")
        .and_then(|a| a.get(SIG_ANNOTATION))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ImagerError::SignatureVerificationFailed(format!(
                "Manifest has no '{}' annotation — image is not signed",
                SIG_ANNOTATION
            ))
        })?
        .to_string();

    let config_digest = extract_config_digest(&manifest_value)?;

    let sig_bytes = Base64Url::decode_vec(&sig_b64).map_err(|e| {
        ImagerError::SignatureVerificationFailed(format!(
            "Failed to decode signature annotation: {}",
            e
        ))
    })?;

    let pubkey_der = parse_pem_public_key(pem)?;

    let public_key =
        signature::UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_ASN1, &pubkey_der);

    public_key
        .verify(config_digest.as_bytes(), &sig_bytes)
        .map_err(|_| {
            ImagerError::SignatureVerificationFailed(
                "Signature verification failed: image was not signed by the trusted key"
                    .to_string(),
            )
        })?;

    Ok(())
}

/// Parse a PEM-encoded ECDSA P-256 public key and return the raw SubjectPublicKeyInfo DER bytes.
pub(crate) fn parse_pem_public_key(pem: &str) -> Result<Vec<u8>> {
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
            "No public key data found in PEM".to_string(),
        ));
    }

    let spki = base64ct::Base64::decode_vec(&b64).map_err(|e| {
        ImagerError::SignatureVerificationFailed(format!(
            "Failed to decode public key from PEM: {}",
            e
        ))
    })?;

    const POINT_OFFSET: usize = 26; // 2 (outer SEQ tag+len) + 21 (AlgId) + 2 (BIT STRING tag+len) + 1 (unused bits)
    const POINT_LEN: usize = 65; // 0x04 || x[32] || y[32]

    if spki.len() < POINT_OFFSET + POINT_LEN {
        return Err(ImagerError::SignatureVerificationFailed(
            "Public key SPKI DER is too short to contain a P-256 point".to_string(),
        ));
    }

    if spki[POINT_OFFSET] != 0x04 {
        return Err(ImagerError::SignatureVerificationFailed(
            "Public key is not an uncompressed EC point (expected 0x04 prefix)".to_string(),
        ));
    }

    Ok(spki[POINT_OFFSET..POINT_OFFSET + POINT_LEN].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_hex_empty() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_sha256_hex_hello() {
        assert_eq!(
            sha256_hex(b"hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

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
        assert!(matches!(
            verify_blob_digest(data, digest).unwrap_err(),
            ImagerError::DigestMismatch { .. }
        ));
    }

    #[test]
    fn test_verify_blob_digest_unsupported_algorithm() {
        let data = b"hello";
        let digest = "md5:abcdef";
        assert!(matches!(
            verify_blob_digest(data, digest).unwrap_err(),
            ImagerError::DigestMismatch { .. }
        ));
    }

    #[test]
    fn test_verify_local_digest_ok() {
        assert!(
            verify_local_digest(
                b"hello",
                "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
                std::path::Path::new("/fake")
            )
            .is_ok()
        );
    }

    #[test]
    fn test_verify_local_digest_ok_no_prefix() {
        assert!(
            verify_local_digest(
                b"hello",
                "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
                std::path::Path::new("/fake")
            )
            .is_ok()
        );
    }

    #[test]
    fn test_verify_local_digest_mismatch() {
        assert!(matches!(
            verify_local_digest(
                b"hello",
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                std::path::Path::new("/fake")
            )
            .unwrap_err(),
            ImagerError::DigestMismatch { .. }
        ));
    }

    #[test]
    fn test_parse_pem_valid() {
        // Real P-256 public key in PKCS#8 SubjectPublicKeyInfo PEM format.
        let pem = "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVDS8kndtUxfYwqGcX2Dw2spTvR44\nt/4lr1W4h75GrFa0zqJwfH9v9oLH5Er0joEKk29+Dya7ZHXDGRiDGoJeYw==\n-----END PUBLIC KEY-----\n";
        let result = parse_pem_public_key(pem);
        assert!(result.is_ok(), "expected Ok, got: {:?}", result);
        let bytes = result.unwrap();
        assert_eq!(bytes.len(), 65, "expected 65-byte uncompressed point");
        assert_eq!(bytes[0], 0x04, "expected uncompressed point prefix 0x04");
    }

    #[test]
    fn test_parse_pem_empty() {
        let pem = "-----BEGIN PUBLIC KEY-----\n-----END PUBLIC KEY-----\n";
        assert!(parse_pem_public_key(pem).is_err());
    }

    #[test]
    fn test_parse_pem_no_markers() {
        assert!(parse_pem_public_key("not a pem file").is_err());
    }

    /// check_signature extracts config.digest and verifies the signature against it.
    #[tokio::test]
    async fn test_check_signature_config_digest_roundtrip() {
        use base64ct::{Base64Url, Encoding};
        use ring::rand::SystemRandom;
        use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair};

        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        let key_pair =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng)
                .unwrap();

        let config_digest =
            "sha256:f572bca63a6f63ee16e3ff053a27f8b0afaa510bd9a474b4412c48ec8351c225";
        let sig = key_pair.sign(&rng, config_digest.as_bytes()).unwrap();
        let sig_b64 = Base64Url::encode_string(sig.as_ref());

        // Build a manifest with the annotation and a config.digest field.
        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "config": {
                "mediaType": "application/vnd.oci.image.config.v1+json",
                "digest": config_digest,
                "size": 100
            },
            "layers": [],
            "annotations": { SIG_ANNOTATION: sig_b64 }
        });
        let manifest_json = serde_json::to_string(&manifest).unwrap();

        // Build a minimal PKCS#8 SubjectPublicKeyInfo PEM from the raw public key bytes.
        let pub_raw = key_pair.public_key().as_ref();
        // Wrap uncompressed EC point in SubjectPublicKeyInfo for P-256.
        let spki = build_p256_spki(pub_raw);
        let pub_pem = format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
            base64ct::Base64::encode_string(&spki)
        );

        let result = check_signature(&manifest_json, Some(&pub_pem)).await;
        assert!(result.is_ok(), "expected Ok, got: {:?}", result);
    }

    /// Builds a minimal SubjectPublicKeyInfo DER wrapping a raw P-256 uncompressed public key.
    fn build_p256_spki(pub_raw: &[u8]) -> Vec<u8> {
        // OID for id-ecPublicKey + OID for P-256 (prime256v1)
        let algorithm: &[u8] = &[
            0x30, 0x13, // SEQUENCE
            0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, // OID id-ecPublicKey
            0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, // OID prime256v1
        ];
        let bit_string_len = 1 + pub_raw.len(); // leading 0x00 unused-bits byte
        let content_len = algorithm.len() + 2 + bit_string_len; // 2 = tag+len for BIT STRING
        let mut der = Vec::new();
        der.push(0x30); // SEQUENCE
        der.push(content_len as u8);
        der.extend_from_slice(algorithm);
        der.push(0x03); // BIT STRING
        der.push(bit_string_len as u8);
        der.push(0x00); // unused bits
        der.extend_from_slice(pub_raw);
        der
    }
}
