//! OCI manifest signature verification and PEM key parsing.

use base64ct::{Base64Url, Encoding as _};
use p256::ecdsa::{Signature as EcdsaSignature, SigningKey, VerifyingKey};
use p256::elliptic_curve::pkcs8::{DecodePrivateKey as _, DecodePublicKey as _};
use serde_json::Value;
use signature::Verifier as _;

use crate::error::{KociError, Result};
use crate::sign::{SIG_ANNOTATION, manifest_signing_payload};

/// Parse a PKCS#8 PEM-encoded ECDSA P-256 private key.
pub(crate) fn parse_pem_private_key(pem: &str) -> Result<SigningKey> {
    SigningKey::from_pkcs8_pem(pem).map_err(|error| {
        KociError::SignatureVerificationFailed(format!(
            "Failed to parse ECDSA P-256 private key (must be PKCS#8 '-----BEGIN PRIVATE KEY-----'): {error}"
        ))
    })
}

/// Parse a PEM-encoded ECDSA P-256 public key.
pub(crate) fn parse_pem_public_key(pem: &str) -> Result<VerifyingKey> {
    VerifyingKey::from_public_key_pem(pem).map_err(|error| {
        KociError::SignatureVerificationFailed(format!(
            "Failed to parse ECDSA P-256 public key (must be '-----BEGIN PUBLIC KEY-----' SPKI): {error}"
        ))
    })
}

/// Check the `SIG_ANNOTATION` annotation on the manifest against the provided public key.
pub(crate) fn check_signature(manifest_json: &str, pubkey_pem: Option<&str>) -> Result<()> {
    let Some(pem) = pubkey_pem else {
        eprintln!(
            "WARNING: No public key provided - manifest signature verification is DISABLED. \
             This image has not been authenticated and may have been tampered with."
        );
        return Ok(());
    };

    let manifest_value: Value = serde_json::from_str(manifest_json).map_err(|error| {
        KociError::SignatureVerificationFailed(format!("Failed to parse manifest JSON: {error}"))
    })?;

    let sig_b64 = manifest_value
        .get("annotations")
        .and_then(|annotations| annotations.get(SIG_ANNOTATION))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            KociError::SignatureVerificationFailed(format!(
                "Manifest has no '{SIG_ANNOTATION}' annotation - image is not signed"
            ))
        })?
        .to_owned();

    let digest = manifest_signing_payload(manifest_json)?;

    let sig_bytes = Base64Url::decode_vec(&sig_b64).map_err(|error| {
        KociError::SignatureVerificationFailed(format!(
            "Failed to decode signature annotation: {error}"
        ))
    })?;

    let verifying_key = parse_pem_public_key(pem)?;
    let signature = EcdsaSignature::from_der(&sig_bytes).map_err(|error| {
        KociError::SignatureVerificationFailed(format!(
            "Failed to decode ASN.1 DER signature annotation: {error}"
        ))
    })?;

    verifying_key
        .verify(digest.as_bytes(), &signature)
        .map_err(|error| {
            KociError::SignatureVerificationFailed(format!(
                "Signature verification failed: image was not signed by the trusted key: {error}"
            ))
        })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use base64ct::Base64;
    use getrandom::SysRng;
    use p256::ecdsa::SigningKey;
    use p256::elliptic_curve::Generate as _;
    use p256::elliptic_curve::pkcs8::{EncodePrivateKey as _, EncodePublicKey as _, LineEnding};
    use p256::elliptic_curve::sec1::ToSec1Point as _;

    use super::*;

    const TEST_PUBKEY_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVDS8kndtUxfYwqGcX2Dw2spTvR44\nt/4lr1W4h75GrFa0zqJwfH9v9oLH5Er0joEKk29+Dya7ZHXDGRiDGoJeYw==\n-----END PUBLIC KEY-----\n";

    fn generate_test_key() -> SigningKey {
        SigningKey::try_generate_from_rng(&mut SysRng).expect("generate test key")
    }

    fn sign_test_digest(key: &SigningKey, digest: &str) -> String {
        let signature: p256::ecdsa::Signature = signature::Signer::sign(key, digest.as_bytes());

        base64ct::Base64Url::encode_string(signature.to_der().as_ref())
    }

    fn public_key_pem(key: &SigningKey) -> String {
        key.verifying_key()
            .to_public_key_pem(LineEnding::LF)
            .expect("encode public key PEM")
    }

    #[tokio::test]
    async fn check_signature_manifest_digest_roundtrip() {
        // ARRANGE
        let key = generate_test_key();

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
        let manifest_bare_json = serde_json::to_string(&manifest_bare).expect("serialize manifest");

        let digest = manifest_signing_payload(&manifest_bare_json)
            .expect("compute manifest signing payload");
        let sig_b64 = sign_test_digest(&key, &digest);

        let mut manifest_signed = manifest_bare.clone();
        manifest_signed
            .as_object_mut()
            .expect("test manifest is an object")
            .insert(
                "annotations".to_owned(),
                serde_json::json!({ SIG_ANNOTATION: sig_b64 }),
            );
        let manifest_signed_json =
            serde_json::to_string(&manifest_signed).expect("serialize signed manifest");

        let pub_pem = public_key_pem(&key);

        // ACT
        let result = check_signature(&manifest_signed_json, Some(&pub_pem));

        // ASSERT
        result.expect("verify manifest signature");
    }

    #[tokio::test]
    async fn check_signature_tampered_manifest_fails() {
        // ARRANGE
        let key = generate_test_key();

        let manifest_bare = serde_json::json!({
            "schemaVersion": 2,
            "config": {"digest": "sha256:abc", "size": 1},
            "layers": []
        });
        let manifest_bare_json = serde_json::to_string(&manifest_bare).expect("serialize manifest");
        let digest = manifest_signing_payload(&manifest_bare_json)
            .expect("compute manifest signing payload");
        let sig_b64 = sign_test_digest(&key, &digest);

        let mut tampered = manifest_bare.clone();
        let tampered_object = tampered
            .as_object_mut()
            .expect("test manifest is an object");
        tampered_object.insert(
            "layers".to_owned(),
            serde_json::json!([{"digest":"sha256:evil","size":999}]),
        );
        tampered_object.insert(
            "annotations".to_owned(),
            serde_json::json!({ SIG_ANNOTATION: sig_b64 }),
        );
        let tampered_json = serde_json::to_string(&tampered).expect("serialize tampered manifest");

        let pub_pem = public_key_pem(&key);

        // ACT
        let result = check_signature(&tampered_json, Some(&pub_pem));

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
        let result = check_signature(manifest_json, None);

        // ASSERT
        result.expect("verification should be skipped without a public key");
    }

    #[tokio::test]
    async fn check_signature_requires_annotation_when_pubkey_is_provided() {
        // ARRANGE
        let manifest_json = r#"{"schemaVersion":2,"layers":[]}"#;

        // ACT
        let error = check_signature(manifest_json, Some(TEST_PUBKEY_PEM))
            .expect_err("verification should fail");

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }

    #[tokio::test]
    async fn check_signature_rejects_invalid_base64_annotation() {
        // ARRANGE
        let manifest_json =
            r#"{"schemaVersion":2,"annotations":{"dev.muak.sig":"!!!"},"layers":[]}"#;

        // ACT
        let error = check_signature(manifest_json, Some(TEST_PUBKEY_PEM))
            .expect_err("verification should fail");

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }

    #[test]
    fn check_signature_rejects_invalid_manifest_json() {
        // ARRANGE
        // ACT
        let error = check_signature("not json", Some(TEST_PUBKEY_PEM))
            .expect_err("verification should fail");

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }

    #[test]
    fn check_signature_rejects_invalid_public_key_pem() {
        // ARRANGE
        let manifest_json =
            r#"{"schemaVersion":2,"annotations":{"dev.muak.sig":"AA"},"layers":[]}"#;

        // ACT
        let error = check_signature(
            manifest_json,
            Some("-----BEGIN PUBLIC KEY-----\n!!!\n-----END PUBLIC KEY-----\n"),
        )
        .expect_err("verification should fail");

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }
    #[test]
    fn parse_pem_public_key_parses_spki() {
        // ARRANGE
        let pem = "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVDS8kndtUxfYwqGcX2Dw2spTvR44\nt/4lr1W4h75GrFa0zqJwfH9v9oLH5Er0joEKk29+Dya7ZHXDGRiDGoJeYw==\n-----END PUBLIC KEY-----\n";

        // ACT
        let verifying_key = parse_pem_public_key(pem).expect("parse PEM public key");
        let point = verifying_key.as_affine().to_sec1_point(false);

        // ASSERT
        assert_eq!(
            point.as_bytes().len(),
            65,
            "expected 65-byte uncompressed point"
        );
        assert_eq!(
            point.as_bytes().first(),
            Some(&0x04),
            "expected uncompressed point prefix 0x04"
        );
    }

    #[test]
    fn parse_pem_empty() {
        // ARRANGE
        let pem = "-----BEGIN PUBLIC KEY-----\n-----END PUBLIC KEY-----\n";

        // ACT & ASSERT
        parse_pem_public_key(pem).expect_err("public key parsing should fail");
    }

    #[test]
    fn parse_pem_no_markers() {
        // ARRANGE
        let input = "not a pem file";

        // ACT
        let result = parse_pem_public_key(input);

        // ASSERT
        result.expect_err("public key parsing should fail");
    }

    #[test]
    fn parse_pem_public_key_rejects_short_spki() {
        // ARRANGE
        let pem = "-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----\n";

        // ACT
        let error = parse_pem_public_key(pem).expect_err("public key parsing should fail");

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }

    #[test]
    fn parse_pem_public_key_rejects_compressed_point() {
        // ARRANGE
        let mut spki = vec![0_u8; 26 + 65];
        *spki
            .get_mut(26)
            .expect("SPKI test fixture has point prefix") = 0x02;
        let pem = format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
            Base64::encode_string(&spki)
        );

        // ACT
        let error = parse_pem_public_key(&pem).expect_err("public key parsing should fail");

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }

    #[test]
    fn parse_pem_public_key_rejects_invalid_base64() {
        // ARRANGE
        let pem = "-----BEGIN PUBLIC KEY-----\n!!!\n-----END PUBLIC KEY-----\n";

        // ACT
        let error = parse_pem_public_key(pem).expect_err("public key parsing should fail");

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }
    fn generate_private_key_pem() -> String {
        let key = SigningKey::try_generate_from_rng(&mut SysRng).expect("generate private key");
        key.to_pkcs8_pem(LineEnding::LF)
            .expect("encode private key PEM")
            .to_string()
    }

    #[test]
    fn parse_pem_private_key_accepts_valid_pkcs8() {
        // ARRANGE
        let pem = generate_private_key_pem();

        // ACT
        let key = parse_pem_private_key(&pem).expect("private key parsing should succeed");

        // ASSERT
        assert!(
            !key.verifying_key()
                .as_affine()
                .to_sec1_point(false)
                .as_bytes()
                .is_empty()
        );
    }

    #[test]
    fn parse_pem_private_key_rejects_missing_pem_block() {
        // ARRANGE / ACT
        let error =
            parse_pem_private_key("not a pem file").expect_err("private key parsing should fail");

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }

    #[test]
    fn parse_pem_private_key_rejects_invalid_pkcs8_bytes() {
        // ARRANGE
        let pem = "-----BEGIN PRIVATE KEY-----\nAAECAwQFBgc=\n-----END PRIVATE KEY-----\n";

        // ACT / ASSERT
        let error = parse_pem_private_key(pem).expect_err("private key parsing should fail");
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }

    #[test]
    fn parse_pem_private_key_rejects_invalid_base64() {
        // ARRANGE
        let pem = "-----BEGIN PRIVATE KEY-----\n!!!\n-----END PRIVATE KEY-----\n";

        // ACT / ASSERT
        let error = parse_pem_private_key(pem).expect_err("private key parsing should fail");
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }
}
