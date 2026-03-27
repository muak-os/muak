//! OCI image signing.
//!
//! Signs an OCI image by adding a `dev.muak.sig` annotation to the manifest
//! containing a base64url-encoded ECDSA P-256 DER signature over the manifest's
//! own SHA-256 content digest.

use base64ct::{Base64Url, Encoding};
use hyper::body::Bytes;
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair};

use crate::error::{ImagerError, Result};
use crate::image::{ImageReference, OciManifest};
use crate::oci::auth::fetch_auth_token;
use crate::oci::http::{HttpClient, build_client, put};
use crate::oci::manifest;
use crate::oci::verify::{SIG_ANNOTATION, sha256_hex};

/// Sign an OCI image manifest in the registry.
pub(crate) async fn sign_manifest(reference: &str, privkey_pem: &str) -> Result<()> {
    let image_ref = ImageReference::parse(reference);
    let client = build_client()?;

    let token = fetch_auth_token(&client, &image_ref.registry, &image_ref.name).await?;
    let key_pair = parse_pem_private_key(privkey_pem)?;
    let rng = SystemRandom::new();

    let manifest_url = manifest::build_url(&image_ref, &image_ref.tag);
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
        &image_ref.tag,
    )
    .await
}

/// Compute the canonical payload for signing a manifest JSON string.
pub(crate) fn manifest_signing_payload(manifest_json: &str) -> Result<(String, Vec<u8>)> {
    let mut value: serde_json::Value = serde_json::from_str(manifest_json)
        .map_err(|e| ImagerError::OciParseError(format!("Failed to parse manifest JSON: {}", e)))?;

    if let Some(obj) = value.as_object_mut() {
        let remove_annotations =
            if let Some(annotations) = obj.get_mut("annotations").and_then(|a| a.as_object_mut()) {
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

    let canonical = serde_json::to_vec(&value).map_err(|e| {
        ImagerError::OciParseError(format!("Failed to serialise canonical manifest: {}", e))
    })?;

    let digest = format!("sha256:{}", sha256_hex(&canonical));
    Ok((digest, canonical))
}

/// Recursively sort all JSON object keys in lexicographic order.
fn sort_keys(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<(String, serde_json::Value)> = map
                .iter_mut()
                .map(|(k, v)| {
                    sort_keys(v);
                    (k.clone(), v.clone())
                })
                .collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            *map = entries.into_iter().collect();
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                sort_keys(item);
            }
        }
        _ => {}
    }
}

/// Build a signed manifest: compute the payload, sign it, inject the annotation,
/// and return `(signed_bytes, content_type)`.
fn build_signed_manifest(
    manifest_json: &str,
    key_pair: &EcdsaKeyPair,
    rng: &SystemRandom,
) -> Result<(Bytes, String)> {
    let (digest, _canonical) = manifest_signing_payload(manifest_json)?;

    let sig = key_pair
        .sign(rng, digest.as_bytes())
        .map_err(|_| ImagerError::SignatureVerificationFailed("Signing failed".to_string()))?;
    let sig_b64 = Base64Url::encode_string(sig.as_ref());

    let mut manifest_value: serde_json::Value = serde_json::from_str(manifest_json)
        .map_err(|e| ImagerError::OciParseError(format!("Failed to parse manifest JSON: {}", e)))?;

    manifest_value
        .as_object_mut()
        .ok_or_else(|| ImagerError::InvalidOciFormat("Manifest is not a JSON object".to_string()))?
        .entry("annotations")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .ok_or_else(|| {
            ImagerError::InvalidOciFormat("Manifest annotations is not a JSON object".to_string())
        })?
        .insert(
            SIG_ANNOTATION.to_string(),
            serde_json::Value::String(sig_b64),
        );

    let content_type = manifest_value
        .get("mediaType")
        .and_then(|v| v.as_str())
        .unwrap_or("application/vnd.oci.image.manifest.v1+json")
        .to_string();

    let signed_bytes = serde_json::to_vec(&manifest_value).map_err(|e| {
        ImagerError::OciParseError(format!("Failed to serialise signed manifest: {}", e))
    })?;

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

/// Parse a PKCS#8 PEM-encoded ECDSA P-256 private key.
fn parse_pem_private_key(pem: &str) -> Result<EcdsaKeyPair> {
    let mut b64 = String::new();
    let mut in_block = false;

    for line in pem.lines() {
        let line = line.trim();
        if line == "-----BEGIN PRIVATE KEY-----" {
            in_block = true;
            continue;
        }
        if line == "-----END PRIVATE KEY-----" {
            break;
        }
        if in_block {
            b64.push_str(line);
        }
    }

    if b64.is_empty() {
        return Err(ImagerError::SignatureVerificationFailed(
            "No private key data found in PEM (expected PKCS#8 '-----BEGIN PRIVATE KEY-----')"
                .to_string(),
        ));
    }

    let der = base64ct::Base64::decode_vec(&b64).map_err(|e| {
        ImagerError::SignatureVerificationFailed(format!(
            "Failed to decode private key from PEM: {}",
            e
        ))
    })?;

    EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &der, &SystemRandom::new()).map_err(
        |_| {
            ImagerError::SignatureVerificationFailed(
                "Failed to parse ECDSA P-256 private key (must be PKCS#8 format)".to_string(),
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use ring::signature::{
        ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair, UnparsedPublicKey,
    };

    use super::*;
    use crate::oci::verify::sha256_hex;

    #[test]
    fn base64url_roundtrip() {
        // ARRANGE
        let original = b"\x30\x44\x02\x20\xde\xad\xbe\xef";

        // ACT
        let encoded = Base64Url::encode_string(original);
        let decoded = Base64Url::decode_vec(&encoded).unwrap();

        // ASSERT
        assert_eq!(original.as_ref(), decoded.as_slice());
    }

    #[test]
    fn manifest_signing_payload_strips_sig_annotation() {
        // ARRANGE
        let manifest_json = r#"{"schemaVersion":2,"annotations":{"dev.muak.sig":"oldsig","other":"keep"},"layers":[]}"#;

        // ACT
        let (digest, canonical) = manifest_signing_payload(manifest_json).unwrap();

        // ASSERT
        let canonical_str = std::str::from_utf8(&canonical).unwrap();
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
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        let key_pair =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng)
                .unwrap();
        let manifest_json = r#"{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json","config":{"digest":"sha256:abc","size":1},"layers":[]}"#;

        // ACT
        let (signed_bytes, _content_type) =
            build_signed_manifest(manifest_json, &key_pair, &rng).unwrap();
        let signed_value: serde_json::Value = serde_json::from_slice(&signed_bytes).unwrap();
        let sig_b64 = signed_value["annotations"][SIG_ANNOTATION]
            .as_str()
            .expect("annotation must be present");
        let sig_bytes = Base64Url::decode_vec(sig_b64).unwrap();
        let (digest, _) = manifest_signing_payload(manifest_json).unwrap();
        let pub_key = UnparsedPublicKey::new(
            &ring::signature::ECDSA_P256_SHA256_ASN1,
            key_pair.public_key().as_ref(),
        );

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
        let (digest1, _) = manifest_signing_payload(manifest_json).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(manifest_json).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .entry("annotations")
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
            .as_object_mut()
            .unwrap()
            .insert(
                SIG_ANNOTATION.to_string(),
                serde_json::Value::String("somesig".to_string()),
            );
        let signed_json = serde_json::to_string(&value).unwrap();

        // ACT
        let (digest2, _) = manifest_signing_payload(&signed_json).unwrap();

        // ASSERT
        assert_eq!(
            digest1, digest2,
            "stripping the annotation must give the same payload on re-sign"
        );
    }
}
