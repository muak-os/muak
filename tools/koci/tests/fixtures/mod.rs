use core::error::Error;
use core::str;
use std::io::{Error as IoError, ErrorKind};

use base64ct::{Base64, Base64Url, Encoding as _};
use flate2::Compression;
use flate2::write::GzEncoder;
use getrandom::SysRng;
use koci::arch;
use p256::ecdsa::SigningKey;
use p256::elliptic_curve::Generate as _;
use p256::elliptic_curve::pkcs8::LineEnding;
use p256::elliptic_curve::pkcs8::{
    DecodePrivateKey as _, EncodePrivateKey as _, EncodePublicKey as _,
};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};
use tar::{Builder, Header};

const SIG_ANNOTATION: &str = "dev.muak.sig";

pub(crate) struct TestKeys {
    pub(crate) private_key_pem: String,
    pub(crate) public_key_pem: String,
}

pub(crate) fn generate_test_keys() -> Result<TestKeys, Box<dyn Error>> {
    let key = SigningKey::try_generate_from_rng(&mut SysRng)
        .map_err(|_error| IoError::other("failed to generate ECDSA test key"))?;

    let private_key_pem = key
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|_error| IoError::other("failed to encode ECDSA test key"))?
        .to_string();
    let public_key_pem = key
        .verifying_key()
        .to_public_key_pem(LineEnding::LF)
        .map_err(|_error| IoError::other("failed to encode ECDSA test public key"))?;

    Ok(TestKeys {
        private_key_pem,
        public_key_pem,
    })
}

pub(crate) fn manifest_json(
    layer_digest: &str,
    layer_size: usize,
) -> Result<Vec<u8>, serde_json::Error> {
    manifest_with_layers_json(&[(
        layer_digest,
        layer_size,
        "application/vnd.oci.image.layer.v1.tar+gzip",
    )])
}

pub(crate) fn manifest_with_layers_json(
    layers: &[(&str, usize, &str)],
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {
            "mediaType": "application/vnd.oci.image.config.v1+json",
            "digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            "size": 1,
        },
        "layers": layers.iter().map(|&(digest, size, media_type)| {
            json!({
                "mediaType": media_type,
                "digest": digest,
                "size": size,
            })
        }).collect::<Vec<_>>(),
    }))
}

pub(crate) fn minimal_manifest_json() -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "layers": [],
    }))
}

pub(crate) fn manifest_without_media_type_json() -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "layers": [],
    }))
}

pub(crate) fn manifest_with_invalid_annotations_json() -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "annotations": [],
        "layers": [],
    }))
}

pub(crate) fn index_json(manifest_digests: &[&str]) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.index.v1+json",
        "manifests": manifest_digests.iter().map(|digest| {
            json!({
                "digest": digest,
                "platform": {
                    "architecture": arch::host().as_str(),
                    "os": "linux",
                }
            })
        }).collect::<Vec<_>>(),
    }))
}

pub(crate) fn index_for_arches_json(
    manifests: &[(&str, &str, &str)],
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.index.v1+json",
        "manifests": manifests.iter().map(|&(digest, arch, os)| {
            json!({
                "digest": digest,
                "platform": {
                    "architecture": arch,
                    "os": os,
                }
            })
        }).collect::<Vec<_>>(),
    }))
}

pub(crate) fn signed_manifest_json(
    manifest_json: &[u8],
    private_key_pem: &str,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let manifest_json = str::from_utf8(manifest_json)?;
    let mut value: Value = serde_json::from_str(manifest_json)?;
    let digest = manifest_signing_digest(manifest_json)?;
    let key = parse_private_key_pem(private_key_pem)?;
    let signature: p256::ecdsa::Signature = signature::Signer::sign(&key, digest.as_bytes());
    let signature = signature.to_der();
    let signature = Base64Url::encode_string(signature.as_ref());

    let object = value
        .as_object_mut()
        .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "manifest is not a JSON object"))?;
    let annotations = object
        .entry("annotations")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| {
            IoError::new(
                ErrorKind::InvalidData,
                "manifest annotations is not a JSON object",
            )
        })?;
    annotations.insert(SIG_ANNOTATION.to_owned(), Value::String(signature));

    Ok(serde_json::to_vec(&value)?)
}

pub(crate) fn with_annotation_json(
    manifest_json: &[u8],
    key: &str,
    value: &str,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut value_json: Value = serde_json::from_slice(manifest_json)?;
    let object = value_json
        .as_object_mut()
        .ok_or_else(|| IoError::new(ErrorKind::InvalidData, "manifest is not a JSON object"))?;
    let annotations = object
        .entry("annotations")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| {
            IoError::new(
                ErrorKind::InvalidData,
                "manifest annotations is not a JSON object",
            )
        })?;
    annotations.insert(key.to_owned(), Value::String(value.to_owned()));

    Ok(serde_json::to_vec(&value_json)?)
}

pub(crate) fn layer_archive(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, Box<dyn Error>> {
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut archive = Builder::new(encoder);

    for &(path, bytes) in entries {
        let mut header = Header::new_gnu();
        header.set_size(u64::try_from(bytes.len())?);
        header.set_mode(0o644);
        header.set_cksum();
        archive.append_data(&mut header, path, bytes)?;
    }

    let encoder = archive.into_inner()?;
    Ok(encoder.finish()?)
}

#[must_use]
pub(crate) fn sha256_digest(bytes: &[u8]) -> String {
    format!(
        "sha256:{}",
        base16ct::lower::encode_string(Sha256::digest(bytes).as_ref())
    )
}

fn manifest_signing_digest(manifest_json: &str) -> Result<String, Box<dyn Error>> {
    let mut value: Value = serde_json::from_str(manifest_json)?;

    if let Some(object) = value.as_object_mut() {
        let remove_annotations = if let Some(annotations) =
            object.get_mut("annotations").and_then(Value::as_object_mut)
        {
            annotations.remove(SIG_ANNOTATION);
            annotations.is_empty()
        } else {
            false
        };

        if remove_annotations {
            object.remove("annotations");
        }
    }

    sort_keys(&mut value);
    Ok(sha256_digest(&serde_json::to_vec(&value)?))
}

fn parse_private_key_pem(pem: &str) -> Result<SigningKey, Box<dyn Error>> {
    let mut body = String::new();
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
            body.push_str(line);
        }
    }

    if body.is_empty() {
        return Err(IoError::new(ErrorKind::InvalidData, "missing private key PEM body").into());
    }

    let der = Base64::decode_vec(&body)?;
    let key = SigningKey::from_pkcs8_der(&der)
        .map_err(|_error| IoError::new(ErrorKind::InvalidData, "invalid ECDSA private key"))?;
    Ok(key)
}

fn sort_keys(value: &mut Value) {
    if let Some(map) = value.as_object_mut() {
        let mut entries: Vec<(String, Value)> = map
            .iter_mut()
            .map(|(key, value)| {
                sort_keys(value);
                (key.clone(), value.clone())
            })
            .collect();
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        *map = entries.into_iter().collect();
        return;
    }

    if let Some(values) = value.as_array_mut() {
        for value in values {
            sort_keys(value);
        }
    }
}
