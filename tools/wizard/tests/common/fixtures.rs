//! OCI image fixture builders.

use std::collections::HashMap;

use flate2::Compression;
use flate2::write::GzEncoder;
use serde_json::json;
use sha2::{Digest as _, Sha256};
use tar::{Builder, Header};

use crate::common::server::{HttpResponse, Routes, get};

/// An OCI image made of one gzip layer with the given file entries.
pub struct FixtureImage {
    pub manifest: Vec<u8>,
    pub layers: Vec<(String, Vec<u8>)>,
}

/// Builds a single-layer OCI image from `(path, bytes)` entries in order,
/// carrying a `dev.muak.sizes` annotation with each entry's byte size.
pub fn build_image(entries: &[(&str, &[u8])]) -> FixtureImage {
    let layer = layer_archive(entries);
    let digest = sha256_digest(&layer);
    let sizes: HashMap<String, u64> = entries
        .iter()
        .map(|&(path, bytes)| (path.to_owned(), u64::try_from(bytes.len()).unwrap_or(0)))
        .collect();
    let manifest = serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "annotations": {
            "dev.muak.sizes": serde_json::to_string(&sizes).expect("serialize sizes"),
        },
        "config": {
            "mediaType": "application/vnd.oci.image.config.v1+json",
            "digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            "size": 1,
        },
        "layers": [{
            "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
            "digest": digest,
            "size": layer.len(),
        }],
    }))
    .expect("serialize manifest");
    FixtureImage {
        manifest,
        layers: vec![(digest, layer)],
    }
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
/// reproducible across runs.
pub fn random_bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed;
    let mut bytes = Vec::with_capacity(len);
    for _ in 0..len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        bytes.push(u8::try_from(state & 0xff).unwrap_or(0));
    }
    bytes
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for &byte in bytes {
        let high = HEX.get(usize::from(byte >> 4)).copied().unwrap_or(0);
        let low = HEX.get(usize::from(byte & 0x0f)).copied().unwrap_or(0);
        encoded.push(char::from(high));
        encoded.push(char::from(low));
    }
    encoded
}
