//! OCI image fixture builders.

use std::collections::HashMap;
use std::io::Read as _;

use flate2::Compression;
use flate2::write::GzEncoder;
use serde_json::json;
use sha2::{Digest as _, Sha256};
use tar::{Builder, Header};

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

/// Builds a single-platform image served as one plain (uncompressed tar) layer.
pub fn build_plain_image(entries: &[(&str, &[u8])]) -> FixtureImage {
    build_layer_image(entries, "application/vnd.oci.image.layer.v1.tar")
}

/// Builds a legacy single-member gzip tar layer image with no features.
pub fn build_legacy_image(entries: &[(&str, &[u8])]) -> FixtureImage {
    let layer = layer_archive(entries);
    build_manifest_image(&layer, "application/vnd.oci.image.layer.v1.tar+gzip")
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

/// Wraps `layer` into a manifest without any annotations.
fn build_manifest_image(layer: &[u8], media_type: &str) -> FixtureImage {
    let digest = sha256_digest(layer);
    let manifest = serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {
            "mediaType": "application/vnd.oci.image.config.v1+json",
            "digest": config_digest(),
            "size": config_blob().len(),
        },
        "layers": [{
            "mediaType": media_type,
            "digest": digest,
            "size": layer.len(),
        }],
    }))
    .expect("serialize manifest");

    FixtureImage {
        manifest,
        layers: vec![(digest, layer.to_vec())],
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

pub fn layer_archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut archive = Builder::new(encoder);

    for &(path, bytes) in entries {
        let mut header = Header::new_gnu();
        header.set_size(u64::try_from(bytes.len()).unwrap_or(0));
        header.set_mode(0o644);
        header.set_cksum();
        archive
            .append_data(&mut header, path, bytes)
            .expect("append fixture entry");
    }

    let encoder = archive.into_inner().expect("finish fixture archive");
    encoder.finish().expect("finish fixture gzip")
}

pub fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex_encode(Sha256::digest(bytes).as_ref()))
}

/// Deterministic pseudo-random bytes (xorshift) so fixture images are
pub fn random_bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed;
    let mut bytes = Vec::with_capacity(len);
    for _ in 0..len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        bytes.push((state & 0xff) as u8);
    }

    bytes
}

/// Lowercase hex encoding of `bytes`.
pub fn hex_encode(bytes: &[u8]) -> String {
    let lookup = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(lookup[usize::from(byte >> 4)] as char);
        out.push(lookup[usize::from(byte & 0x0f)] as char);
    }

    out
}
