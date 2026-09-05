use core::error::Error;
use std::io::Error as IoError;

use flate2::Compression;
use flate2::write::GzEncoder;
use getrandom::SysRng;
use koci::arch;
use p256::ecdsa::SigningKey;
use p256::elliptic_curve::Generate as _;
use p256::elliptic_curve::pkcs8::LineEnding;
use p256::elliptic_curve::pkcs8::{EncodePrivateKey as _, EncodePublicKey as _};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};
use tar::{Builder, Header};

pub(crate) const SIG_ANNOTATION: &str = "dev.muak.sig";

pub(crate) const SIZES_ANNOTATION: &str = "dev.muak.sizes";

const GZIP_LAYER_MEDIA_TYPE: &str = "application/vnd.oci.image.layer.v1.tar+gzip";

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
    manifest_with_layers_json(&[(layer_digest, layer_size, GZIP_LAYER_MEDIA_TYPE)])
}

pub(crate) fn manifest_with_layers_json(
    layers: &[(&str, usize, &str)],
) -> Result<Vec<u8>, serde_json::Error> {
    manifest_with_layers_and_annotations_json(layers, &[])
}

/// Build a manifest JSON for a single gzip layer with pre-set annotations.
pub(crate) fn annotated_manifest_json(
    layer_digest: &str,
    layer_size: usize,
    annotations: &[(&str, &str)],
) -> Result<Vec<u8>, serde_json::Error> {
    manifest_with_layers_and_annotations_json(
        &[(layer_digest, layer_size, GZIP_LAYER_MEDIA_TYPE)],
        annotations,
    )
}

fn manifest_with_layers_and_annotations_json(
    layers: &[(&str, usize, &str)],
    annotations: &[(&str, &str)],
) -> Result<Vec<u8>, serde_json::Error> {
    let mut manifest = json!({
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
    });

    if !annotations.is_empty() {
        let annotations_map = annotations
            .iter()
            .map(|&(key, value)| (key.to_owned(), Value::from(value)))
            .collect::<Map<String, Value>>();
        manifest
            .as_object_mut()
            .expect("manifest fixture must be a JSON object")
            .insert("annotations".to_owned(), Value::Object(annotations_map));
    }

    serde_json::to_vec(&manifest)
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
