use std::error::Error;
use std::io::{Error as IoError, ErrorKind};

use base64ct::{Base64, Base64Url, Encoding};
use flate2::Compression;
use flate2::write::GzEncoder;
use ring::digest;
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair};
use serde_json::{Map, Value, json};
use tar::{Builder, Header};

const SIG_ANNOTATION: &str = "dev.muak.sig";

pub fn host_oci_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        other => other,
    }
}

pub struct TestKeys {
    pub private_key_pem: String,
    pub public_key_pem: String,
}

pub fn generate_test_keys() -> Result<TestKeys, Box<dyn Error>> {
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng)
        .map_err(|_| IoError::other("failed to generate ECDSA test key"))?;
    let key_pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng)
        .map_err(|_| IoError::other("failed to parse generated ECDSA test key"))?;

    let private_key_pem = format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
        Base64::encode_string(pkcs8.as_ref())
    );
    let public_key_pem = format!(
        "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
        Base64::encode_string(&build_p256_spki(key_pair.public_key().as_ref()))
    );

    Ok(TestKeys {
        private_key_pem,
        public_key_pem,
    })
}

pub fn manifest_json(layer_digest: &str, layer_size: usize) -> Result<Vec<u8>, serde_json::Error> {
    manifest_with_layers_json(&[(
        layer_digest,
        layer_size,
        "application/vnd.oci.image.layer.v1.tar+gzip",
    )])
}

pub fn manifest_with_layers_json(
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
        "layers": layers.iter().map(|(digest, size, media_type)| {
            json!({
                "mediaType": media_type,
                "digest": digest,
                "size": size,
            })
        }).collect::<Vec<_>>(),
    }))
}

pub fn minimal_manifest_json() -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "layers": [],
    }))
}

pub fn manifest_without_media_type_json() -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "layers": [],
    }))
}

pub fn manifest_with_invalid_annotations_json() -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "annotations": [],
        "layers": [],
    }))
}

pub fn index_json(manifest_digests: &[&str]) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.index.v1+json",
        "manifests": manifest_digests.iter().map(|digest| {
            json!({
                "digest": digest,
                "platform": {
                    "architecture": host_oci_arch(),
                    "os": "linux",
                }
            })
        }).collect::<Vec<_>>(),
    }))
}

pub fn index_for_arches_json(
    manifests: &[(&str, &str, &str)],
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.index.v1+json",
        "manifests": manifests.iter().map(|(digest, arch, os)| {
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

pub fn signed_manifest_json(
    manifest_json: &[u8],
    private_key_pem: &str,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let manifest_json = std::str::from_utf8(manifest_json)?;
    let mut value: Value = serde_json::from_str(manifest_json)?;
    let digest = manifest_signing_digest(manifest_json)?;
    let key_pair = parse_private_key_pem(private_key_pem)?;
    let rng = SystemRandom::new();
    let signature = key_pair
        .sign(&rng, digest.as_bytes())
        .map_err(|_| IoError::other("failed to sign manifest fixture"))?;
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
    annotations.insert(SIG_ANNOTATION.to_string(), Value::String(signature));

    Ok(serde_json::to_vec(&value)?)
}

pub fn layer_archive(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, Box<dyn Error>> {
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut archive = Builder::new(encoder);

    for (path, bytes) in entries {
        let mut header = Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive.append_data(&mut header, path, *bytes)?;
    }

    let encoder = archive.into_inner()?;
    Ok(encoder.finish()?)
}

pub fn sha256_digest(bytes: &[u8]) -> String {
    format!(
        "sha256:{}",
        hex_encode(digest::digest(&digest::SHA256, bytes).as_ref())
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

fn parse_private_key_pem(pem: &str) -> Result<EcdsaKeyPair, Box<dyn Error>> {
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
    let rng = SystemRandom::new();
    let key_pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &der, &rng)
        .map_err(|_| IoError::new(ErrorKind::InvalidData, "invalid ECDSA private key"))?;
    Ok(key_pair)
}

fn sort_keys(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map
                .iter_mut()
                .map(|(key, value)| {
                    sort_keys(value);
                    (key.clone(), value.clone())
                })
                .collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            *map = entries.into_iter().collect();
        }
        Value::Array(values) => {
            for value in values {
                sort_keys(value);
            }
        }
        _ => {}
    }
}

fn build_p256_spki(public_key: &[u8]) -> Vec<u8> {
    let algorithm: &[u8] = &[
        0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86,
        0x48, 0xce, 0x3d, 0x03, 0x01, 0x07,
    ];
    let bit_string_len = 1 + public_key.len();
    let content_len = algorithm.len() + 2 + bit_string_len;
    let mut der = Vec::with_capacity(2 + content_len);
    der.push(0x30);
    der.push(content_len as u8);
    der.extend_from_slice(algorithm);
    der.push(0x03);
    der.push(bit_string_len as u8);
    der.push(0x00);
    der.extend_from_slice(public_key);
    der
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }

    encoded
}
