//! OCI image signing.

use core::mem;

use base64ct::{Base64Url, Encoding as _};
use hyper::body::Bytes;
use ring::rand::SystemRandom;
use ring::signature::EcdsaKeyPair;

use crate::digest::sha256_hex;
use crate::error::{KociError, Result};
use crate::image::manifest;
use crate::image::{ImageReference, OciManifest};
use crate::registry::auth::fetch_auth_token;
use crate::registry::http::{HttpClient, build_client, put};
use crate::sign::key::parse_pem_private_key;

pub(crate) mod key;
pub(crate) mod verify;

/// Annotation key used to store the image signature.
pub(crate) const SIG_ANNOTATION: &str = "dev.muak.sig";

/// Sign an OCI image manifest in the registry.
///
/// # Errors
///
/// Returns an error if the manifest cannot be fetched, signed, or pushed.
pub async fn manifest(reference: &str, privkey_pem: &str) -> Result<()> {
    let image_ref = ImageReference::parse(reference);
    let client = build_client();

    let token = fetch_auth_token(&client, &image_ref.registry, &image_ref.name).await?;
    let key_pair = parse_pem_private_key(privkey_pem)?;
    let rng = SystemRandom::new();

    let manifest_url = manifest::build_url(&image_ref, &image_ref.manifest_ref);
    let manifest_json = manifest::fetch(&client, &manifest_url, token.as_deref()).await?;
    let parsed: OciManifest = manifest::parse(&manifest_json)?;

    if !parsed.manifests.is_empty() {
        for descriptor in &parsed.manifests {
            let platform_url = manifest::build_url(&image_ref, &descriptor.digest);
            let platform_json = manifest::fetch(&client, &platform_url, token.as_deref()).await?;
            let (signed_bytes, content_type) =
                build_signed_manifest(&platform_json, &key_pair, &rng)?;
            push_manifest(
                &client,
                &image_ref,
                token.as_deref(),
                signed_bytes,
                &content_type,
                &descriptor.digest,
            )
            .await?;
        }
    }

    let (signed_bytes, content_type) = build_signed_manifest(&manifest_json, &key_pair, &rng)?;

    push_manifest(
        &client,
        &image_ref,
        token.as_deref(),
        signed_bytes,
        &content_type,
        &image_ref.manifest_ref,
    )
    .await
}

/// Compute the canonical digest for signing a manifest JSON string.
pub(crate) fn manifest_signing_payload(manifest_json: &str) -> Result<String> {
    let canonical = canonicalize_manifest(manifest_json)?;

    Ok(format!("sha256:{}", sha256_hex(&canonical)))
}

/// Strip the signature annotation and produce canonical (sorted-key) JSON bytes.
fn canonicalize_manifest(manifest_json: &str) -> Result<Vec<u8>> {
    let mut value: serde_json::Value = match serde_json::from_str(manifest_json) {
        Ok(value) => value,
        Err(error) => {
            return Err(KociError::OciParseError(format!(
                "Failed to parse manifest JSON: {error}"
            )));
        }
    };

    if let Some(obj) = value.as_object_mut() {
        let remove_annotations = if let Some(annotations) = obj
            .get_mut("annotations")
            .and_then(serde_json::Value::as_object_mut)
        {
            annotations.remove(SIG_ANNOTATION);
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
fn sort_keys(value: &mut serde_json::Value) {
    match *value {
        serde_json::Value::Object(ref mut map) => {
            let mut entries = mem::take(map).into_iter().collect::<Vec<_>>();
            for &mut (_, ref mut entry_value) in &mut entries {
                sort_keys(entry_value);
            }
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            *map = entries.into_iter().collect();
        }
        serde_json::Value::Array(ref mut arr) => {
            for item in arr.iter_mut() {
                sort_keys(item);
            }
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {}
    }
}

/// Build a signed manifest: compute the payload, sign it and inject the annotation.
pub(crate) fn build_signed_manifest(
    manifest_json: &str,
    key_pair: &EcdsaKeyPair,
    rng: &SystemRandom,
) -> Result<(Bytes, String)> {
    let digest = manifest_signing_payload(manifest_json)?;

    let sig = match key_pair.sign(rng, digest.as_bytes()) {
        Ok(signature) => signature,
        Err(error) => {
            return Err(KociError::SignatureVerificationFailed(format!(
                "Signing failed: {error}"
            )));
        }
    };
    let sig_b64 = Base64Url::encode_string(sig.as_ref());

    let mut manifest_value: serde_json::Value = match serde_json::from_str(manifest_json) {
        Ok(value) => value,
        Err(error) => {
            return Err(KociError::OciParseError(format!(
                "Failed to parse manifest JSON: {error}"
            )));
        }
    };

    manifest_value
        .as_object_mut()
        .ok_or_else(|| KociError::InvalidOciFormat("Manifest is not a JSON object".to_owned()))?
        .entry("annotations")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .ok_or_else(|| {
            KociError::InvalidOciFormat("Manifest annotations is not a JSON object".to_owned())
        })?
        .insert(
            SIG_ANNOTATION.to_owned(),
            serde_json::Value::String(sig_b64),
        );

    let content_type = manifest_value
        .get("mediaType")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("application/vnd.oci.image.manifest.v1+json")
        .to_owned();

    let signed_bytes = serde_json::to_vec(&manifest_value)?;

    Ok((Bytes::from(signed_bytes), content_type))
}

/// Push a manifest to the registry via PUT.
async fn push_manifest(
    client: &HttpClient,
    image_ref: &ImageReference,
    token: Option<&str>,
    body: Bytes,
    content_type: &str,
    reference: &str,
) -> Result<()> {
    let url = manifest::build_url(image_ref, reference);
    put(client, &url, token, content_type, body).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use core::str;

    use ring::signature::{
        ECDSA_P256_SHA256_ASN1, ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair as _,
        UnparsedPublicKey,
    };

    use super::*;
    use crate::digest::sha256_hex;

    fn generate_test_key_pair(rng: &SystemRandom) -> EcdsaKeyPair {
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, rng)
            .expect("generate ECDSA test key");
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), rng)
            .expect("parse generated ECDSA test key")
    }

    fn decode_base64url(input: &str) -> Vec<u8> {
        Base64Url::decode_vec(input).expect("decode base64url test value")
    }

    fn decode_utf8(bytes: &[u8]) -> &str {
        str::from_utf8(bytes).expect("decode UTF-8 test value")
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
    fn manifest_signing_payload_strips_sig_annotation() {
        // ARRANGE
        let manifest_json = r#"{"schemaVersion":2,"annotations":{"dev.muak.sig":"oldsig","other":"keep"},"layers":[]}"#;

        // ACT
        let digest =
            manifest_signing_payload(manifest_json).expect("compute manifest signing payload");
        let canonical = canonicalize_manifest(manifest_json).expect("canonicalize manifest");

        // ASSERT
        let canonical_str = decode_utf8(&canonical);
        assert!(
            !canonical_str.contains("dev.muak.sig"),
            "canonical bytes must not contain the sig annotation"
        );
        assert!(
            canonical_str.contains("keep"),
            "unrelated annotations must be preserved"
        );
        assert_eq!(digest, format!("sha256:{}", sha256_hex(&canonical)));
    }

    #[test]
    fn build_signed_manifest_roundtrip() {
        // ARRANGE
        let rng = SystemRandom::new();
        let key_pair = generate_test_key_pair(&rng);
        let manifest_json = r#"{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","config":{"digest":"sha256:abc","size":1},"layers":[]}"#;

        // ACT
        let (signed_bytes, _content_type) =
            build_signed_manifest(manifest_json, &key_pair, &rng).expect("build signed manifest");
        let signed_value: serde_json::Value =
            serde_json::from_slice(&signed_bytes).expect("parse signed manifest");
        let sig_b64 = signed_value
            .get("annotations")
            .and_then(|annotations| annotations.get(SIG_ANNOTATION))
            .and_then(serde_json::Value::as_str)
            .expect("signed manifest must include the signature annotation");
        let sig_bytes = decode_base64url(sig_b64);
        let digest =
            manifest_signing_payload(manifest_json).expect("compute manifest signing payload");
        let pub_key =
            UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, key_pair.public_key().as_ref());

        // ASSERT
        assert!(
            pub_key.verify(digest.as_bytes(), &sig_bytes).is_ok(),
            "signature must verify against the canonical manifest digest"
        );
    }

    #[test]
    fn manifest_signing_payload_idempotent() {
        // ARRANGE
        let manifest_json = r#"{"schemaVersion":2,"layers":[]}"#;
        let digest1 = manifest_signing_payload(manifest_json)
            .expect("compute initial manifest signing payload");
        let mut value: serde_json::Value =
            serde_json::from_str(manifest_json).expect("parse manifest json");
        let annotations = value
            .as_object_mut()
            .expect("manifest payload must be a JSON object")
            .entry("annotations")
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
            .as_object_mut()
            .expect("manifest annotations must be a JSON object");
        annotations.insert(
            SIG_ANNOTATION.to_owned(),
            serde_json::Value::String("somesig".to_owned()),
        );
        let signed_json = serde_json::to_string(&value).expect("serialize signed manifest");

        // ACT
        let digest2 =
            manifest_signing_payload(&signed_json).expect("compute signed manifest payload");

        // ASSERT
        assert_eq!(
            digest1, digest2,
            "stripping the annotation must give the same payload on re-sign"
        );
    }

    #[test]
    fn manifest_signing_payload_removes_empty_annotations_map() {
        // ARRANGE
        let manifest_json =
            r#"{"schemaVersion":2,"annotations":{"dev.muak.sig":"oldsig"},"layers":[]}"#;

        // ACT
        let canonical = canonicalize_manifest(manifest_json).expect("canonicalize manifest");
        let canonical_str = decode_utf8(&canonical);

        // ASSERT
        assert_eq!(canonical_str, r#"{"layers":[],"schemaVersion":2}"#);
    }

    #[test]
    fn manifest_signing_payload_rejects_invalid_json() {
        // ARRANGE / ACT
        let error =
            manifest_signing_payload("not json").expect_err("payload generation should fail");

        // ASSERT
        assert!(matches!(error, KociError::OciParseError(_)));
    }

    #[test]
    fn build_signed_manifest_rejects_invalid_json() {
        // ARRANGE
        let rng = SystemRandom::new();
        let key_pair = generate_test_key_pair(&rng);

        // ACT
        let error =
            build_signed_manifest("not json", &key_pair, &rng).expect_err("signing should fail");

        // ASSERT
        assert!(matches!(error, KociError::OciParseError(_)));
    }

    #[test]
    fn build_signed_manifest_rejects_non_object_annotations() {
        // ARRANGE
        let rng = SystemRandom::new();
        let key_pair = generate_test_key_pair(&rng);
        let manifest_json = r#"{"schemaVersion":2,"annotations":[],"layers":[]}"#;

        // ACT
        let error =
            build_signed_manifest(manifest_json, &key_pair, &rng).expect_err("signing should fail");

        // ASSERT
        assert!(matches!(error, KociError::InvalidOciFormat(_)));
    }

    #[tokio::test]
    async fn push_manifest_propagates_put_failures() {
        // ARRANGE
        let client = build_client();
        let image_ref = ImageReference::parse("127.0.0.1:9/repo:test");

        // ACT
        let error = push_manifest(
            &client,
            &image_ref,
            None,
            Bytes::from_static(b"{}"),
            "application/vnd.oci.image.manifest.v1+json",
            "test",
        )
        .await
        .expect_err("push manifest should fail");

        // ASSERT
        assert!(matches!(error, KociError::NetworkError(_)));
    }
}
