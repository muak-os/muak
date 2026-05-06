//! OCI integrity and signature verification.

use base64ct::{Base64Url, Encoding};
use ring::signature;
use serde_json::Value;

use crate::error::{ImagerError, Result};
use crate::oci::sign::manifest_signing_payload;

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

/// Check the `SIG_ANNOTATION` annotation on the manifest against the provided public key.
pub(crate) async fn check_signature(manifest_json: &str, pubkey_pem: Option<&str>) -> Result<()> {
    let Some(pem) = pubkey_pem else {
        eprintln!(
            "WARNING: No public key provided — manifest signature verification is DISABLED. \
             This image has not been authenticated and may have been tampered with."
        );
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

    let (digest, _canonical) = manifest_signing_payload(manifest_json)?;

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
        .verify(digest.as_bytes(), &sig_bytes)
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
    use base64ct::Encoding;
    use ring::rand::SystemRandom;
    use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair};

    use super::*;

    fn must<T, E: std::fmt::Display>(result: std::result::Result<T, E>, context: &str) -> T {
        match result {
            Ok(value) => value,
            Err(error) => panic!("{context}: {error}"),
        }
    }

    fn generate_test_key_pair(rng: &SystemRandom) -> Result<EcdsaKeyPair> {
        let pkcs8 =
            EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, rng).map_err(|_| {
                ImagerError::SignatureVerificationFailed(
                    "failed to generate ECDSA test key".to_string(),
                )
            })?;

        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), rng).map_err(
            |_| {
                ImagerError::SignatureVerificationFailed(
                    "failed to parse generated ECDSA test key".to_string(),
                )
            },
        )
    }

    fn sign_test_digest(
        key_pair: &EcdsaKeyPair,
        rng: &SystemRandom,
        digest: &str,
    ) -> Result<String> {
        let sig = key_pair.sign(rng, digest.as_bytes()).map_err(|_| {
            ImagerError::SignatureVerificationFailed(
                "failed to sign manifest digest in test".to_string(),
            )
        })?;

        Ok(base64ct::Base64Url::encode_string(sig.as_ref()))
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

    #[test]
    fn sha256_hex_empty() {
        // ACT & ASSERT
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_hex_hello() {
        // ACT & ASSERT
        assert_eq!(
            sha256_hex(b"hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn verify_blob_digest_ok() {
        // ARRANGE
        let data = b"hello";
        let digest = "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

        // ACT
        let result = verify_blob_digest(data, digest);

        // ASSERT
        assert!(result.is_ok());
    }

    #[test]
    fn verify_blob_digest_unsupported_algorithm() {
        // ARRANGE
        let data = b"hello";
        let digest = "md5:abcdef";

        // ACT
        let result = verify_blob_digest(data, digest);

        // ASSERT
        assert!(matches!(result, Err(ImagerError::DigestMismatch { .. })));
    }

    #[test]
    fn parse_pem_valid() {
        // ARRANGE
        let pem = "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVDS8kndtUxfYwqGcX2Dw2spTvR44\nt/4lr1W4h75GrFa0zqJwfH9v9oLH5Er0joEKk29+Dya7ZHXDGRiDGoJeYw==\n-----END PUBLIC KEY-----\n";

        // ACT
        let bytes = must(parse_pem_public_key(pem), "parse PEM public key");

        // ASSERT
        assert_eq!(bytes.len(), 65, "expected 65-byte uncompressed point");
        assert_eq!(bytes[0], 0x04, "expected uncompressed point prefix 0x04");
    }

    #[test]
    fn parse_pem_empty() {
        // ARRANGE
        let pem = "-----BEGIN PUBLIC KEY-----\n-----END PUBLIC KEY-----\n";

        // ACT & ASSERT
        assert!(parse_pem_public_key(pem).is_err());
    }

    #[test]
    fn parse_pem_no_markers() {
        // ARRANGE
        let input = "not a pem file";

        // ACT
        let result = parse_pem_public_key(input);

        // ASSERT
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn check_signature_manifest_digest_roundtrip() {
        // ARRANGE

        let rng = SystemRandom::new();
        let key_pair = must(generate_test_key_pair(&rng), "generate test key pair");

        let manifest_bare = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "config": {
                "mediaType": "application/vnd.oci.image.config.v1+json",
                "digest": "sha256:f572bca63a6f63ee16e3ff053a27f8b0afaa510bd9a474b4412c48ec8351c225",
                "size": 100
            },
            "layers": []
        });
        let manifest_bare_json = must(serde_json::to_string(&manifest_bare), "serialize manifest");

        let (digest, _) = must(
            manifest_signing_payload(&manifest_bare_json),
            "compute manifest signing payload",
        );
        let sig_b64 = must(
            sign_test_digest(&key_pair, &rng, &digest),
            "sign manifest digest",
        );

        let mut manifest_signed = manifest_bare.clone();
        manifest_signed["annotations"] = serde_json::json!({ SIG_ANNOTATION: sig_b64 });
        let manifest_signed_json = must(
            serde_json::to_string(&manifest_signed),
            "serialize signed manifest",
        );

        let pub_raw = key_pair.public_key().as_ref();
        let spki = build_p256_spki(pub_raw);
        let pub_pem = format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
            base64ct::Base64::encode_string(&spki)
        );

        // ACT
        must(
            check_signature(&manifest_signed_json, Some(&pub_pem)).await,
            "verify manifest signature",
        );
    }

    #[tokio::test]
    async fn check_signature_tampered_manifest_fails() {
        // ARRANGE
        let rng = SystemRandom::new();
        let key_pair = must(generate_test_key_pair(&rng), "generate test key pair");

        let manifest_bare = serde_json::json!({
            "schemaVersion": 2,
            "config": {"digest": "sha256:abc", "size": 1},
            "layers": []
        });
        let manifest_bare_json = must(serde_json::to_string(&manifest_bare), "serialize manifest");
        let (digest, _) = must(
            manifest_signing_payload(&manifest_bare_json),
            "compute manifest signing payload",
        );
        let sig_b64 = must(
            sign_test_digest(&key_pair, &rng, &digest),
            "sign manifest digest",
        );

        let mut tampered = manifest_bare.clone();
        tampered["layers"] = serde_json::json!([{"digest":"sha256:evil","size":999}]);
        tampered["annotations"] = serde_json::json!({ SIG_ANNOTATION: sig_b64 });
        let tampered_json = must(
            serde_json::to_string(&tampered),
            "serialize tampered manifest",
        );

        let pub_raw = key_pair.public_key().as_ref();
        let spki = build_p256_spki(pub_raw);
        let pub_pem = format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
            base64ct::Base64::encode_string(&spki)
        );

        // ACT
        let result = check_signature(&tampered_json, Some(&pub_pem)).await;

        // ASSERT
        assert!(
            result.is_err(),
            "tampered manifest must NOT verify successfully"
        );
    }

    #[tokio::test]
    async fn check_signature_without_pubkey_is_allowed() {
        // ARRANGE
        let manifest_json = r#"{"schemaVersion":2,"layers":[]}"#;

        // ACT
        let result = check_signature(manifest_json, None).await;

        // ASSERT
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn check_signature_requires_annotation_when_pubkey_is_provided() {
        // ARRANGE
        let pem = "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVDS8kndtUxfYwqGcX2Dw2spTvR44\nt/4lr1W4h75GrFa0zqJwfH9v9oLH5Er0joEKk29+Dya7ZHXDGRiDGoJeYw==\n-----END PUBLIC KEY-----\n";
        let manifest_json = r#"{"schemaVersion":2,"layers":[]}"#;

        // ACT
        let error = match check_signature(manifest_json, Some(pem)).await {
            Ok(()) => panic!("signature verification unexpectedly succeeded"),
            Err(error) => error,
        };

        // ASSERT
        assert!(matches!(error, ImagerError::SignatureVerificationFailed(_)));
    }

    #[tokio::test]
    async fn check_signature_rejects_invalid_base64_annotation() {
        // ARRANGE
        let pem = "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVDS8kndtUxfYwqGcX2Dw2spTvR44\nt/4lr1W4h75GrFa0zqJwfH9v9oLH5Er0joEKk29+Dya7ZHXDGRiDGoJeYw==\n-----END PUBLIC KEY-----\n";
        let manifest_json =
            r#"{"schemaVersion":2,"annotations":{"dev.muak.sig":"!!!"},"layers":[]}"#;

        // ACT
        let error = match check_signature(manifest_json, Some(pem)).await {
            Ok(()) => panic!("signature verification unexpectedly succeeded"),
            Err(error) => error,
        };

        // ASSERT
        assert!(matches!(error, ImagerError::SignatureVerificationFailed(_)));
    }

    #[test]
    fn parse_pem_public_key_rejects_short_spki() {
        // ARRANGE
        let pem = "-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----\n";

        // ACT
        let error = match parse_pem_public_key(pem) {
            Ok(_) => panic!("public key unexpectedly parsed"),
            Err(error) => error,
        };

        // ASSERT
        assert!(matches!(error, ImagerError::SignatureVerificationFailed(_)));
    }

    #[test]
    fn parse_pem_public_key_rejects_compressed_point() {
        // ARRANGE
        let mut spki = vec![0u8; 26 + 65];
        spki[26] = 0x02;
        let pem = format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
            base64ct::Base64::encode_string(&spki)
        );

        // ACT
        let error = match parse_pem_public_key(&pem) {
            Ok(_) => panic!("public key unexpectedly parsed"),
            Err(error) => error,
        };

        // ASSERT
        assert!(matches!(error, ImagerError::SignatureVerificationFailed(_)));
    }
}
