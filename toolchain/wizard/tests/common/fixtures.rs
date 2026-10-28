//! OCI image fixture builders.

use serde_json::json;
use sha2::{Digest as _, Sha256};

use crate::common::server::{HttpResponse, Routes, get};

/// An OCI image whose layers are eStargz blobs.
pub struct FixtureImage {
    pub manifest: Vec<u8>,
    pub layers: Vec<(String, Vec<u8>)>,
}

/// Builds a single-platform image from `(path, bytes)` entries in order,
/// rewritten as ONE sorted eStargz layer carrying its TOC and footer.
pub fn build_image(entries: &[(&str, &[u8])]) -> FixtureImage {
    build_layer_image(entries, "application/vnd.oci.image.layer.v1.tar+gzip")
}

/// Builds a single-platform image with one sorted eStargz layer per entry slice.
pub fn build_layers_image(layers: &[&[(&str, &[u8])]]) -> FixtureImage {
    let packed: Vec<Vec<u8>> = layers.iter().map(|entries| stargz_layer(entries)).collect();
    let slices: Vec<&[u8]> = packed.iter().map(Vec::as_slice).collect();

    build_manifest_layers(&slices, "application/vnd.oci.image.layer.v1.tar+gzip")
}

/// Builds a single-platform image whose layer is the given blob and type.
fn build_layer_image(entries: &[(&str, &[u8])], media_type: &str) -> FixtureImage {
    let packed = stargz_layer(entries);
    build_manifest_image(&packed, media_type)
}

/// Architecture reported by the single-arch fixture config.
const FIXTURE_ARCH: &str = "amd64";

/// Image config blob served for single-arch wizard fixtures.
pub fn config_blob() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "architecture": FIXTURE_ARCH,
        "os": "linux",
    }))
    .expect("serialize config blob")
}

/// Digest of [`config_blob`], matching the bytes the mock registry serves.
pub fn config_digest() -> String {
    sha256_digest(&config_blob())
}

/// Wraps one layer into a manifest without any annotations.
fn build_manifest_image(layer: &[u8], media_type: &str) -> FixtureImage {
    build_manifest_layers(&[layer], media_type)
}

/// Wraps `layers` into a manifest without any annotations.
fn build_manifest_layers(layers: &[&[u8]], media_type: &str) -> FixtureImage {
    let mut descriptors = Vec::with_capacity(layers.len());
    let mut blobs = Vec::with_capacity(layers.len());
    for layer in layers {
        let digest = sha256_digest(layer);
        descriptors.push(json!({
            "mediaType": media_type,
            "digest": &digest,
            "size": layer.len(),
        }));
        blobs.push((digest, layer.to_vec()));
    }

    let manifest = serde_json::to_vec(&json!({
        "schemaVersion": 2_i32,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {
            "mediaType": "application/vnd.oci.image.config.v1+json",
            "digest": config_digest(),
            "size": config_blob().len(),
        },
        "layers": descriptors,
    }))
    .expect("serialize manifest");

    FixtureImage {
        manifest,
        layers: blobs,
    }
}

/// Encodes entries into one eStargz layer blob through the koci writer.
fn stargz_layer(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let packed: Vec<koci::stargz::Entry> = entries
        .iter()
        .map(|&(path, bytes)| koci::stargz::Entry {
            path: path.to_owned(),
            mode: 0o644,
            source: koci::stargz::Source::Bytes(bytes.to_vec()),
        })
        .collect();
    koci::stargz::layer(&packed)
        .expect("build eStargz fixture layer")
        .bytes
}

/// Serves an image under `GET /v2/{repo}/manifests/{tag}` and its blobs.
pub fn install_image(routes: &mut Routes, repo: &str, tag: &str, image: &FixtureImage) {
    let (key, response) = get(
        format!("/v2/{repo}/manifests/{tag}"),
        HttpResponse::json(image.manifest.clone()),
    );
    routes.insert(key, response);
    for item in &image.layers {
        let (key, response) = get(
            format!("/v2/{repo}/blobs/{}", item.0),
            HttpResponse::octet_stream(item.1.clone()),
        );
        routes.insert(key, response);
    }
}

pub fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex_encode(Sha256::digest(bytes).as_ref()))
}

/// Deterministic pseudo-random bytes (xorshift) so fixture images stay
/// reproducible while differing from each other.
pub fn random_bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed;
    let mut bytes = Vec::with_capacity(len);
    for _ in 0..len {
        state ^= state << 13_u64;
        state ^= state >> 7_u64;
        state ^= state << 17_u64;
        bytes.push(u8::try_from(state & 0xff).unwrap_or_default());
    }

    bytes
}

/// Lowercase hex encoding of `bytes`.
pub fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        out.push(char::from_digit(u32::from(byte >> 4_u8), 16).unwrap_or('0'));
        out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }

    out
}
