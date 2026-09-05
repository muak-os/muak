//! Manifest payload canonicalization, signing, and signature verification.

use core::mem;

use base64ct::{Base64Url, Encoding as _};
use hyper::body::Bytes;
use p256::ecdsa::{Signature as EcdsaSignature, SigningKey, VerifyingKey};
use p256::elliptic_curve::pkcs8::{DecodePrivateKey as _, DecodePublicKey as _};
use serde_json::Value;
use signature::{Signer as _, Verifier as _};

use crate::annotations::Verification;
use crate::digest::sha256_hex;
use crate::error::{KociError, Result};
use crate::image::manifest;

/// Sign the canonical manifest payload and inject it as `annotation`.
pub(crate) fn inject(
    manifest_json: &str,
    key: &SigningKey,
    annotation: &str,
) -> Result<(Bytes, String)> {
    let digest = signing_payload(manifest_json, annotation)?;
    let signature: EcdsaSignature = key.sign(digest.as_bytes());
    let sig_b64 = Base64Url::encode_string(signature.to_der().as_ref());

    manifest::with_annotation(manifest_json, annotation, &sig_b64)
}

/// Check a manifest's signature annotation against the trusted public key.
pub(crate) fn check_signature(
    manifest_json: &str,
    verification: Option<&Verification<'_>>,
) -> Result<()> {
    let Some(verification) = verification else {
        eprintln!(
            "WARNING: No public key provided - manifest signature verification is DISABLED. \
             This image has not been authenticated and may have been tampered with."
        );
        return Ok(());
    };
    let annotation = verification.sig_annotation;

    let manifest_value: Value = serde_json::from_str(manifest_json).map_err(|error| {
        KociError::SignatureVerificationFailed(format!("Failed to parse manifest JSON: {error}"))
    })?;

    let sig_b64 = manifest_value
        .get("annotations")
        .and_then(|annotations| annotations.get(annotation))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            KociError::SignatureVerificationFailed(format!(
                "Manifest has no '{annotation}' annotation - image is not signed"
            ))
        })?
        .to_owned();

    let digest = signing_payload(manifest_json, annotation)?;
    let sig_bytes = Base64Url::decode_vec(&sig_b64).map_err(|error| {
        KociError::SignatureVerificationFailed(format!(
            "Failed to decode signature annotation: {error}"
        ))
    })?;

    let verifying_key = parse_pem_public_key(verification.pubkey_pem)?;
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

/// Parse a PKCS#8 PEM-encoded ECDSA P-256 private key.
pub(crate) fn parse_pem_private_key(pem: &str) -> Result<SigningKey> {
    SigningKey::from_pkcs8_pem(pem).map_err(|error| {
        KociError::SignatureVerificationFailed(format!(
            "Failed to parse ECDSA P-256 private key (must be PKCS#8 '-----BEGIN PRIVATE KEY-----'): {error}"
        ))
    })
}

/// Compute the canonical `sha256:` digest of the manifest with `annotation` stripped.
fn signing_payload(manifest_json: &str, annotation: &str) -> Result<String> {
    let canonical = canonicalize_manifest(manifest_json, annotation)?;

    Ok(format!("sha256:{}", sha256_hex(&canonical)))
}

/// Strip the signature annotation and produce canonical (sorted-key) JSON bytes.
fn canonicalize_manifest(manifest_json: &str, sig_annotation: &str) -> Result<Vec<u8>> {
    let mut value: Value = match serde_json::from_str(manifest_json) {
        Ok(value) => value,
        Err(error) => {
            return Err(KociError::OciParseError(format!(
                "Failed to parse manifest JSON: {error}"
            )));
        }
    };

    if let Some(obj) = value.as_object_mut() {
        let remove_annotations =
            if let Some(annotations) = obj.get_mut("annotations").and_then(Value::as_object_mut) {
                annotations.remove(sig_annotation);
                annotations.is_empty()
            } else {
                false
            };
        if remove_annotations {
            obj.remove("annotations");
        }
    }

    sort_keys(&mut value);

    serde_json::to_vec(&value).map_err(Into::into)
}

/// Recursively sort all JSON object keys in lexicographic order.
fn sort_keys(value: &mut Value) {
    match *value {
        Value::Object(ref mut map) => {
            let mut entries = mem::take(map).into_iter().collect::<Vec<_>>();
            for &mut (_, ref mut entry_value) in &mut entries {
                sort_keys(entry_value);
            }
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            *map = entries.into_iter().collect();
        }
        Value::Array(ref mut arr) => {
            for item in arr.iter_mut() {
                sort_keys(item);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

/// Parse a PEM-encoded ECDSA P-256 public key.
fn parse_pem_public_key(pem: &str) -> Result<VerifyingKey> {
    VerifyingKey::from_public_key_pem(pem).map_err(|error| {
        KociError::SignatureVerificationFailed(format!(
            "Failed to parse ECDSA P-256 public key (must be '-----BEGIN PUBLIC KEY-----' SPKI): {error}"
        ))
    })
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
    use crate::digest::sha256_hex;

    const SIG_ANNOTATION: &str = "dev.muak.sig";

    const TEST_PUBKEY_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVDS8kndtUxfYwqGcX2Dw2spTvR44\nt/4lr1W4h75GrFa0zqJwfH9v9oLH5Er0joEKk29+Dya7ZHXDGRiDGoJeYw==\n-----END PUBLIC KEY-----\n";

    fn generate_test_key() -> SigningKey {
        SigningKey::try_generate_from_rng(&mut SysRng).expect("generate test key")
    }

    fn decode_base64url(input: &str) -> Vec<u8> {
        Base64Url::decode_vec(input).expect("decode base64url test value")
    }

    fn decode_utf8(bytes: &[u8]) -> &str {
        core::str::from_utf8(bytes).expect("decode UTF-8 test value")
    }

    fn sign_test_digest(key: &SigningKey, digest: &str) -> String {
        let signature: EcdsaSignature = key.sign(digest.as_bytes());

        Base64Url::encode_string(signature.to_der().as_ref())
    }

    fn public_key_pem(key: &SigningKey) -> String {
        key.verifying_key()
            .to_public_key_pem(LineEnding::LF)
            .expect("encode public key PEM")
    }

    fn verification(pubkey_pem: &str) -> Verification<'_> {
        Verification {
            pubkey_pem,
            sig_annotation: SIG_ANNOTATION,
        }
    }

    #[test]
    fn base64url_roundtrip() {
        // ARRANGE
        let original = b"\x30\x44\x02\x20\xde\xad\xbe\xef";

        // ACT
        let encoded = Base64Url::encode_string(original);
        let decoded = decode_base64url(&encoded);

        // ASSERT
        assert_eq!(original.as_ref(), decoded.as_slice());
    }

    #[test]
    fn signing_payload_strips_the_signature_annotation() {
        // ARRANGE
        let manifest_json = format!(
            r#"{{"schemaVersion":2,"annotations":{{"{SIG_ANNOTATION}":"oldsig","other":"keep"}},"layers":[]}}"#
        );

        // ACT
        let digest = signing_payload(&manifest_json, SIG_ANNOTATION).expect("compute payload");
        let canonical =
            canonicalize_manifest(&manifest_json, SIG_ANNOTATION).expect("canonicalize manifest");

        // ASSERT
        let canonical_str = decode_utf8(&canonical);
        assert!(
            !canonical_str.contains(SIG_ANNOTATION),
            "canonical bytes must not contain the sig annotation"
        );
        assert!(
            canonical_str.contains("keep"),
            "unrelated annotations must be preserved"
        );
        assert_eq!(digest, format!("sha256:{}", sha256_hex(&canonical)));
    }

    #[test]
    fn inject_signs_the_canonical_manifest_digest() {
        // ARRANGE
        let key = generate_test_key();
        let manifest_json = r#"{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","config":{"digest":"sha256:abc","size":1},"layers":[]}"#;

        // ACT
        let (signed_bytes, _content_type) =
            inject(manifest_json, &key, SIG_ANNOTATION).expect("build signed manifest");
        let signed_value: Value =
            serde_json::from_slice(&signed_bytes).expect("parse signed manifest");
        let sig_b64 = signed_value
            .get("annotations")
            .and_then(|annotations| annotations.get(SIG_ANNOTATION))
            .and_then(Value::as_str)
            .expect("signed manifest must include the signature annotation");
        let sig_bytes = decode_base64url(sig_b64);
        let digest = signing_payload(manifest_json, SIG_ANNOTATION).expect("compute payload");
        let signature = EcdsaSignature::from_der(&sig_bytes).expect("parse ASN.1 DER signature");
        let verifying_key = VerifyingKey::from_sec1_bytes(
            key.verifying_key()
                .as_affine()
                .to_sec1_point(false)
                .as_bytes(),
        )
        .expect("parse public key point");

        // ASSERT
        let result = verifying_key.verify(digest.as_bytes(), &signature);
        assert!(
            result.is_ok(),
            "signature must verify against the canonical manifest digest"
        );
    }

    #[test]
    fn signing_payload_is_idempotent_across_re_signing() {
        // ARRANGE
        let manifest_json = r#"{"schemaVersion":2,"layers":[]}"#;
        let digest1 =
            signing_payload(manifest_json, SIG_ANNOTATION).expect("compute initial payload");
        let mut value: Value = serde_json::from_str(manifest_json).expect("parse manifest json");
        let annotations = value
            .as_object_mut()
            .expect("manifest payload must be a JSON object")
            .entry("annotations")
            .or_insert_with(|| Value::Object(serde_json::Map::new()))
            .as_object_mut()
            .expect("manifest annotations must be a JSON object");
        annotations.insert(
            SIG_ANNOTATION.to_owned(),
            Value::String("somesig".to_owned()),
        );
        let signed_json = serde_json::to_string(&value).expect("serialize signed manifest");

        // ACT
        let digest2 = signing_payload(&signed_json, SIG_ANNOTATION).expect("compute re-signed");

        // ASSERT
        assert_eq!(
            digest1, digest2,
            "stripping the annotation must give the same payload on re-sign"
        );
    }

    #[test]
    fn canonicalize_removes_an_empty_annotations_map() {
        // ARRANGE
        let manifest_json = format!(
            r#"{{"schemaVersion":2,"annotations":{{"{SIG_ANNOTATION}":"oldsig"}},"layers":[]}}"#
        );

        // ACT
        let canonical =
            canonicalize_manifest(&manifest_json, SIG_ANNOTATION).expect("canonicalize manifest");
        let canonical_str = decode_utf8(&canonical);

        // ASSERT
        assert_eq!(canonical_str, r#"{"layers":[],"schemaVersion":2}"#);
    }

    #[test]
    fn signing_payload_rejects_invalid_json() {
        // ARRANGE / ACT
        let error = signing_payload("not json", SIG_ANNOTATION).expect_err("payload should fail");

        // ASSERT
        assert!(matches!(error, KociError::OciParseError(_)));
    }

    #[test]
    fn inject_rejects_invalid_json() {
        // ARRANGE
        let key = generate_test_key();

        // ACT
        let error = inject("not json", &key, SIG_ANNOTATION).expect_err("signing should fail");

        // ASSERT
        assert!(matches!(error, KociError::OciParseError(_)));
    }

    #[test]
    fn inject_rejects_non_object_annotations() {
        // ARRANGE
        let key = generate_test_key();
        let manifest_json = r#"{"schemaVersion":2,"annotations":[],"layers":[]}"#;

        // ACT
        let error = inject(manifest_json, &key, SIG_ANNOTATION).expect_err("signing should fail");

        // ASSERT
        assert!(matches!(error, KociError::InvalidOciFormat(_)));
    }

    #[tokio::test]
    async fn check_signature_verifies_a_signed_manifest_digest() {
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

        let digest = signing_payload(&manifest_bare_json, SIG_ANNOTATION)
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
        let result = check_signature(&manifest_signed_json, Some(&verification(&pub_pem)));

        // ASSERT
        result.expect("verify manifest signature");
    }

    #[tokio::test]
    async fn check_signature_rejects_a_tampered_manifest() {
        // ARRANGE
        let key = generate_test_key();

        let manifest_bare = serde_json::json!({
            "schemaVersion": 2,
            "config": {"digest": "sha256:abc", "size": 1},
            "layers": []
        });
        let manifest_bare_json = serde_json::to_string(&manifest_bare).expect("serialize manifest");
        let digest = signing_payload(&manifest_bare_json, SIG_ANNOTATION)
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
        let result = check_signature(&tampered_json, Some(&verification(&pub_pem)));

        // ASSERT
        assert!(
            result.is_err(),
            "tampered manifest must NOT verify successfully"
        );
    }

    #[tokio::test]
    async fn check_signature_skips_verification_without_requirements() {
        // ARRANGE
        let manifest_json = r#"{"schemaVersion":2,"layers":[]}"#;

        // ACT
        let result = check_signature(manifest_json, None);

        // ASSERT
        result.expect("verification should be skipped without a public key");
    }

    #[tokio::test]
    async fn check_signature_requires_the_annotation_when_verifying() {
        // ARRANGE
        let manifest_json = r#"{"schemaVersion":2,"layers":[]}"#;

        // ACT
        let error = check_signature(manifest_json, Some(&verification(TEST_PUBKEY_PEM)))
            .expect_err("verification should fail");

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }

    #[tokio::test]
    async fn check_signature_rejects_invalid_base64_annotation() {
        // ARRANGE
        let manifest_json = format!(
            r#"{{"schemaVersion":2,"annotations":{{"{SIG_ANNOTATION}":"!!!"}},"layers":[]}}"#
        );

        // ACT
        let error = check_signature(&manifest_json, Some(&verification(TEST_PUBKEY_PEM)))
            .expect_err("verification should fail");

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }

    #[test]
    fn check_signature_rejects_invalid_manifest_json() {
        // ARRANGE
        // ACT
        let error = check_signature("not json", Some(&verification(TEST_PUBKEY_PEM)))
            .expect_err("verification should fail");

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }

    #[test]
    fn check_signature_rejects_invalid_public_key_pem() {
        // ARRANGE
        let manifest_json = format!(
            r#"{{"schemaVersion":2,"annotations":{{"{SIG_ANNOTATION}":"AA"}},"layers":[]}}"#
        );

        // ACT
        let error = check_signature(
            &manifest_json,
            Some(&verification(
                "-----BEGIN PUBLIC KEY-----\n!!!\n-----END PUBLIC KEY-----\n",
            )),
        )
        .expect_err("verification should fail");

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }

    #[test]
    fn parse_pem_public_key_parses_spki() {
        // ARRANGE
        // ACT
        let verifying_key = parse_pem_public_key(TEST_PUBKEY_PEM).expect("parse PEM public key");
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
            "expected uncompressed point prefix"
        );
    }

    #[test]
    fn parse_pem_public_key_rejects_empty_block() {
        // ARRANGE
        let pem = "-----BEGIN PUBLIC KEY-----\n-----END PUBLIC KEY-----\n";

        // ACT & ASSERT
        parse_pem_public_key(pem).expect_err("public key parsing should fail");
    }

    #[test]
    fn parse_pem_public_key_rejects_missing_markers() {
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

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }

    #[test]
    fn parse_pem_private_key_rejects_invalid_base64() {
        // ARRANGE
        let pem = "-----BEGIN PRIVATE KEY-----\n!!!\n-----END PRIVATE KEY-----\n";

        // ACT / ASSERT
        let error = parse_pem_private_key(pem).expect_err("private key parsing should fail");

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }
}
