//! OCI image signing.
//!
//! Signs an OCI image by adding a `dev.muak.sig` annotation to the manifest
//! containing a base64url-encoded ECDSA P-256 DER signature over the image's
//! config digest string.

use base64ct::{Base64Url, Encoding};
use reqwest::Client;
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair};

use crate::error::{ImagerError, Result};
use crate::image::ImageReference;
use crate::oci::USER_AGENT;
use crate::oci::auth::fetch_auth_token;
use crate::oci::manifest;
use crate::oci::verify::SIG_ANNOTATION;

/// Sign an OCI image manifest in the registry.
///
/// Fetches the manifest at `reference`, signs the config digest string with the
/// ECDSA P-256 private key, and pushes the manifest back with a `dev.muak.sig`
/// annotation containing the base64url-encoded DER signature.
///
/// `privkey_pem` must be a PKCS#8 PEM (`-----BEGIN PRIVATE KEY-----`). Generate with:
/// ```sh
/// openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out cosign.key
/// openssl pkey -pubout -in cosign.key -out cosign.pub
/// ```
pub(crate) async fn sign_manifest(reference: &str, privkey_pem: &str) -> Result<()> {
    let image_ref = ImageReference::parse(reference);
    let client = Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| ImagerError::NetworkError(format!("Failed to create HTTP client: {}", e)))?;

    let token = fetch_auth_token(&client, &image_ref.registry, &image_ref.name).await?;
    let manifest_url = manifest::build_url(&image_ref, &image_ref.tag);
    let manifest_json = manifest::fetch(&client, &manifest_url, token.as_deref()).await?;

    let key_pair = parse_pem_private_key(privkey_pem)?;

    let mut manifest_value: serde_json::Value = serde_json::from_str(&manifest_json)
        .map_err(|e| ImagerError::OciParseError(format!("Failed to parse manifest JSON: {}", e)))?;

    let config_digest = extract_config_digest(&manifest_value)?;

    // Sign the config digest string bytes — stable regardless of manifest JSON formatting.
    let rng = SystemRandom::new();
    let sig = key_pair
        .sign(&rng, config_digest.as_bytes())
        .map_err(|_| ImagerError::SignatureVerificationFailed("Signing failed".to_string()))?;
    let sig_b64 = Base64Url::encode_string(sig.as_ref());

    // Inject the annotation and PUT the manifest back.
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

    let signed_manifest = serde_json::to_vec(&manifest_value)
        .map_err(|e| ImagerError::OciParseError(format!("Failed to serialise manifest: {}", e)))?;

    let content_type = manifest_value
        .get("mediaType")
        .and_then(|v| v.as_str())
        .unwrap_or("application/vnd.oci.image.manifest.v1+json")
        .to_string();

    push_manifest(
        &client,
        &image_ref,
        token.as_deref(),
        &signed_manifest,
        &content_type,
    )
    .await
}

/// Extract the config digest string from a parsed manifest Value.
pub(crate) fn extract_config_digest(manifest_value: &serde_json::Value) -> Result<String> {
    manifest_value
        .get("config")
        .and_then(|c| c.get("digest"))
        .and_then(|d| d.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            ImagerError::InvalidOciFormat("Manifest has no config.digest field".to_string())
        })
}

/// Push a manifest to the registry via PUT.
async fn push_manifest(
    client: &Client,
    image_ref: &ImageReference,
    token: Option<&str>,
    body: &[u8],
    content_type: &str,
) -> Result<()> {
    let url = manifest::build_url(image_ref, &image_ref.tag);
    let mut request = client
        .put(&url)
        .header("Content-Type", content_type)
        .body(body.to_vec());
    if let Some(t) = token {
        request = request.header("Authorization", format!("Bearer {}", t));
    }
    let response = request
        .send()
        .await
        .map_err(|e| ImagerError::NetworkError(format!("PUT manifest failed: {}", e)))?;
    if !response.status().is_success() {
        return Err(ImagerError::NetworkError(format!(
            "PUT manifest returned {}: {}",
            response.status(),
            url
        )));
    }
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
    use super::*;

    #[test]
    fn test_sign_verify_roundtrip() {
        use ring::signature::KeyPair;

        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        let key_pair =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng)
                .unwrap();

        let message = b"sha256:f572bca63a6f63ee16e3ff053a27f8b0afaa510bd9a474b4412c48ec8351c225";
        let sig = key_pair.sign(&rng, message).unwrap();

        let pub_key = ring::signature::UnparsedPublicKey::new(
            &ring::signature::ECDSA_P256_SHA256_ASN1,
            key_pair.public_key().as_ref(),
        );
        assert!(pub_key.verify(message, sig.as_ref()).is_ok());
    }

    #[test]
    fn test_base64url_roundtrip() {
        let original = b"\x30\x44\x02\x20\xde\xad\xbe\xef";
        let encoded = Base64Url::encode_string(original);
        let decoded = Base64Url::decode_vec(&encoded).unwrap();
        assert_eq!(original.as_ref(), decoded.as_slice());
    }

    #[test]
    fn test_extract_config_digest_ok() {
        let manifest: serde_json::Value = serde_json::from_str(
            r#"{"config":{"digest":"sha256:abc123","mediaType":"application/vnd.oci.image.config.v1+json","size":100}}"#,
        )
        .unwrap();
        assert_eq!(extract_config_digest(&manifest).unwrap(), "sha256:abc123");
    }

    #[test]
    fn test_extract_config_digest_missing() {
        let manifest: serde_json::Value = serde_json::from_str(r#"{"layers":[]}"#).unwrap();
        assert!(matches!(
            extract_config_digest(&manifest).unwrap_err(),
            ImagerError::InvalidOciFormat(_)
        ));
    }
}
