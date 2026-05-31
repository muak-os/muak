//! OCI manifest signature verification.

use base64ct::{Base64Url, Encoding as _};
use ring::signature;
use serde_json::Value;

use crate::error::{KociError, Result};
use crate::sign::key::parse_pem_public_key;
use crate::sign::{SIG_ANNOTATION, manifest_signing_payload};

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

    let (digest, _canonical) = manifest_signing_payload(manifest_json)?;

    let sig_bytes = Base64Url::decode_vec(&sig_b64).map_err(|error| {
        KociError::SignatureVerificationFailed(format!(
            "Failed to decode signature annotation: {error}"
        ))
    })?;

    let pubkey_der = parse_pem_public_key(pem)?;

    let public_key =
        signature::UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_ASN1, &pubkey_der);

    public_key
        .verify(digest.as_bytes(), &sig_bytes)
        .map_err(|error| {
            KociError::SignatureVerificationFailed(format!(
                "Signature verification failed: image was not signed by the trusted key: {error}"
            ))
        })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use base64ct::Encoding as _;
    use ring::rand::SystemRandom;
    use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair as _};

    use super::*;

    fn generate_test_key_pair(rng: &SystemRandom) -> EcdsaKeyPair {
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, rng)
            .expect("generate ECDSA test key");

        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), rng)
            .expect("parse generated ECDSA test key")
    }

    fn sign_test_digest(key_pair: &EcdsaKeyPair, rng: &SystemRandom, digest: &str) -> String {
        let sig = key_pair
            .sign(rng, digest.as_bytes())
            .expect("sign manifest digest in test");

        base64ct::Base64Url::encode_string(sig.as_ref())
    }

    /// Builds a minimal `SubjectPublicKeyInfo` DER wrapping a raw P-256 uncompressed public key.
    fn build_p256_spki(pub_raw: &[u8]) -> Vec<u8> {
        let algorithm: &[u8] = &[
            0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
            0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07,
        ];
        let bit_string_len = pub_raw
            .len()
            .checked_add(1)
            .expect("test public key length must fit DER bit string length");
        let content_len = algorithm
            .len()
            .checked_add(2)
            .and_then(|length| length.checked_add(bit_string_len))
            .expect("test public key length must fit DER content length");
        let mut der = Vec::new();
        der.push(0x30);
        der.push(u8::try_from(content_len).expect("test DER content length must fit one byte"));
        der.extend_from_slice(algorithm);
        der.push(0x03);
        der.push(
            u8::try_from(bit_string_len).expect("test DER bit string length must fit one byte"),
        );
        der.push(0x00);
        der.extend_from_slice(pub_raw);
        der
    }

    #[tokio::test]
    async fn check_signature_manifest_digest_roundtrip() {
        // ARRANGE
        let rng = SystemRandom::new();
        let key_pair = generate_test_key_pair(&rng);

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

        let (digest, _) = manifest_signing_payload(&manifest_bare_json)
            .expect("compute manifest signing payload");
        let sig_b64 = sign_test_digest(&key_pair, &rng, &digest);

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

        let pub_raw = key_pair.public_key().as_ref();
        let spki = build_p256_spki(pub_raw);
        let pub_pem = format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
            base64ct::Base64::encode_string(&spki)
        );

        // ACT
        check_signature(&manifest_signed_json, Some(&pub_pem)).expect("verify manifest signature");
    }

    #[tokio::test]
    async fn check_signature_tampered_manifest_fails() {
        // ARRANGE
        let rng = SystemRandom::new();
        let key_pair = generate_test_key_pair(&rng);

        let manifest_bare = serde_json::json!({
            "schemaVersion": 2,
            "config": {"digest": "sha256:abc", "size": 1},
            "layers": []
        });
        let manifest_bare_json = serde_json::to_string(&manifest_bare).expect("serialize manifest");
        let (digest, _) = manifest_signing_payload(&manifest_bare_json)
            .expect("compute manifest signing payload");
        let sig_b64 = sign_test_digest(&key_pair, &rng, &digest);

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

        let pub_raw = key_pair.public_key().as_ref();
        let spki = build_p256_spki(pub_raw);
        let pub_pem = format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
            base64ct::Base64::encode_string(&spki)
        );

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
        let pem = "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVDS8kndtUxfYwqGcX2Dw2spTvR44\nt/4lr1W4h75GrFa0zqJwfH9v9oLH5Er0joEKk29+Dya7ZHXDGRiDGoJeYw==\n-----END PUBLIC KEY-----\n";
        let manifest_json = r#"{"schemaVersion":2,"layers":[]}"#;

        // ACT
        let error =
            check_signature(manifest_json, Some(pem)).expect_err("verification should fail");

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }

    #[tokio::test]
    async fn check_signature_rejects_invalid_base64_annotation() {
        // ARRANGE
        let pem = "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVDS8kndtUxfYwqGcX2Dw2spTvR44\nt/4lr1W4h75GrFa0zqJwfH9v9oLH5Er0joEKk29+Dya7ZHXDGRiDGoJeYw==\n-----END PUBLIC KEY-----\n";
        let manifest_json =
            r#"{"schemaVersion":2,"annotations":{"dev.muak.sig":"!!!"},"layers":[]}"#;

        // ACT
        let error =
            check_signature(manifest_json, Some(pem)).expect_err("verification should fail");

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }

    #[test]
    fn check_signature_rejects_invalid_manifest_json() {
        // ARRANGE
        let pem = "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVDS8kndtUxfYwqGcX2Dw2spTvR44\nt/4lr1W4h75GrFa0zqJwfH9v9oLH5Er0joEKk29+Dya7ZHXDGRiDGoJeYw==\n-----END PUBLIC KEY-----\n";

        // ACT
        let error = check_signature("not json", Some(pem)).expect_err("verification should fail");

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
}
