//! Integration tests for imager OCI pulling and signing.

mod fixtures;
mod registry;

use std::path::Path;
use std::process::Command;

use fixtures::*;
use imager::ImagerError;
use registry::{HttpResponse, MockRegistry, RecordedRequest, get, put};
use serde_json::Value;
use tempfile::TempDir;

fn required_request(registry: &MockRegistry, method: &str, path: &str) -> RecordedRequest {
    match registry
        .request(method, path)
        .expect("read mock registry request log")
    {
        Some(request) => request,
        None => panic!("missing {method} request for {path}"),
    }
}

fn imager_bin() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_imager"))
}

#[tokio::test]
async fn pull_extracts_single_layer_from_local_registry() {
    // ARRANGE
    let layer = layer_archive(&[
        ("etc/motd", b"hello from imager\n"),
        ("usr/share/imager/message.txt", b"integration test\n"),
    ])
    .expect("build layer archive");
    let layer_digest = sha256_digest(&layer);
    let manifest = manifest_json(&layer_digest, layer.len()).expect("build manifest json");

    let registry = MockRegistry::start(std::collections::HashMap::from([
        get("/v2/repo/manifests/test", HttpResponse::json(manifest)),
        get(
            format!("/v2/repo/blobs/{layer_digest}"),
            HttpResponse::octet_stream(layer),
        ),
    ]))
    .expect("start mock registry");
    let output = TempDir::new().expect("create temp dir");

    // ACT
    imager::pull(&registry.reference("repo", "test"), output.path(), None)
        .await
        .expect("pull image");

    // ASSERT
    assert_eq!(
        std::fs::read_to_string(output.path().join("etc/motd")).expect("read motd"),
        "hello from imager\n"
    );
    assert_eq!(
        std::fs::read_to_string(output.path().join("usr/share/imager/message.txt"))
            .expect("read message file"),
        "integration test\n"
    );
}

#[tokio::test]
async fn pull_selects_host_platform_manifest_from_index() {
    // ARRANGE
    let layer = layer_archive(&[("etc/platform", b"selected host manifest\n")])
        .expect("build layer archive");
    let layer_digest = sha256_digest(&layer);
    let selected_manifest_digest =
        "sha256:2222222222222222222222222222222222222222222222222222222222222222";
    let fallback_manifest_digest =
        "sha256:3333333333333333333333333333333333333333333333333333333333333333";

    let index = index_with_fallback_json(selected_manifest_digest, fallback_manifest_digest)
        .expect("build index json");
    let selected_manifest = manifest_json(&layer_digest, layer.len()).expect("build manifest json");
    let fallback_manifest = manifest_json(
        "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        1,
    )
    .expect("build fallback manifest json");

    let registry = MockRegistry::start(std::collections::HashMap::from([
        get("/v2/repo/manifests/test", HttpResponse::index(index)),
        get(
            format!("/v2/repo/manifests/{selected_manifest_digest}"),
            HttpResponse::json(selected_manifest),
        ),
        get(
            format!("/v2/repo/manifests/{fallback_manifest_digest}"),
            HttpResponse::json(fallback_manifest),
        ),
        get(
            format!("/v2/repo/blobs/{layer_digest}"),
            HttpResponse::octet_stream(layer),
        ),
    ]))
    .expect("start mock registry");
    let output = TempDir::new().expect("create temp dir");

    // ACT
    imager::pull(&registry.reference("repo", "test"), output.path(), None)
        .await
        .expect("pull image");

    // ASSERT
    assert_eq!(
        std::fs::read_to_string(output.path().join("etc/platform")).expect("read platform file"),
        "selected host manifest\n"
    );
}

#[tokio::test]
async fn pull_rejects_blob_with_digest_mismatch() {
    // ARRANGE
    let layer =
        layer_archive(&[("etc/invalid", b"digest mismatch\n")]).expect("build layer archive");
    let manifest = manifest_json(
        "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        layer.len(),
    )
    .expect("build manifest json");

    let registry = MockRegistry::start(std::collections::HashMap::from([
        get("/v2/repo/manifests/test", HttpResponse::json(manifest)),
        get(
            "/v2/repo/blobs/sha256:0000000000000000000000000000000000000000000000000000000000000000",
            HttpResponse::octet_stream(layer),
        ),
    ]))
    .expect("start mock registry");
    let output = TempDir::new().expect("create temp dir");

    // ACT
    let error = match imager::pull(&registry.reference("repo", "test"), output.path(), None).await {
        Ok(()) => {
            panic!("pull unexpectedly succeeded");
        }
        Err(error) => error,
    };

    // ASSERT
    assert!(matches!(error, ImagerError::DigestMismatch { .. }));
    assert!(
        std::fs::read_dir(output.path())
            .expect("read output directory")
            .next()
            .is_none()
    );
}

#[tokio::test]
async fn sign_signs_index_and_platform_manifests() {
    // ARRANGE
    let keys = generate_test_keys().expect("generate test keys");
    let top_level_path = "/v2/repo/manifests/test";
    let first_platform_digest =
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let second_platform_digest =
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    let registry = MockRegistry::start(std::collections::HashMap::from([
        get(
            top_level_path,
            HttpResponse::index(
                index_json(&[first_platform_digest, second_platform_digest])
                    .expect("build index json"),
            ),
        ),
        get(
            format!("/v2/repo/manifests/{first_platform_digest}"),
            HttpResponse::json(minimal_manifest_json().expect("build manifest json")),
        ),
        get(
            format!("/v2/repo/manifests/{second_platform_digest}"),
            HttpResponse::json(minimal_manifest_json().expect("build manifest json")),
        ),
        put(
            format!("/v2/repo/manifests/{first_platform_digest}"),
            HttpResponse::ok(),
        ),
        put(
            format!("/v2/repo/manifests/{second_platform_digest}"),
            HttpResponse::ok(),
        ),
        put(top_level_path, HttpResponse::ok()),
    ]))
    .expect("start mock registry");

    // ACT
    imager::sign(&registry.reference("repo", "test"), &keys.private_key_pem)
        .await
        .expect("sign image");

    // ASSERT
    for path in [
        top_level_path.to_string(),
        format!("/v2/repo/manifests/{first_platform_digest}"),
        format!("/v2/repo/manifests/{second_platform_digest}"),
    ] {
        let request = required_request(&registry, "PUT", &path);
        let manifest: Value =
            serde_json::from_slice(&request.body).expect("parse signed manifest body");
        let signature = manifest["annotations"]["dev.muak.sig"].as_str();
        assert!(
            signature.is_some(),
            "signed manifest must include the signature annotation"
        );
    }
}

#[tokio::test]
async fn sign_uses_default_manifest_content_type_when_media_type_is_missing() {
    // ARRANGE
    let keys = generate_test_keys().expect("generate test keys");
    let path = "/v2/repo/manifests/test";
    let registry = MockRegistry::start(std::collections::HashMap::from([
        get(
            path,
            HttpResponse::json(manifest_without_media_type_json().expect("build manifest json")),
        ),
        put(path, HttpResponse::ok()),
    ]))
    .expect("start mock registry");

    // ACT
    imager::sign(&registry.reference("repo", "test"), &keys.private_key_pem)
        .await
        .expect("sign image");

    // ASSERT
    let request = required_request(&registry, "PUT", path);

    assert_eq!(
        request.headers.get("content-type").map(String::as_str),
        Some("application/vnd.oci.image.manifest.v1+json")
    );
}

#[tokio::test]
async fn sign_rejects_invalid_private_key_before_network() {
    // ARRANGE / ACT
    let error = match imager::sign("127.0.0.1:9/repo:test", "not a pem file").await {
        Ok(()) => {
            panic!("sign unexpectedly succeeded");
        }
        Err(error) => error,
    };

    // ASSERT
    assert!(matches!(error, ImagerError::SignatureVerificationFailed(_)));
}

#[tokio::test]
async fn sign_rejects_non_object_manifest_annotations() {
    // ARRANGE
    let keys = generate_test_keys().expect("generate test keys");
    let registry = MockRegistry::start(std::collections::HashMap::from([
        get(
            "/v2/repo/manifests/test",
            HttpResponse::json(
                manifest_with_invalid_annotations_json().expect("build manifest json"),
            ),
        ),
        put("/v2/repo/manifests/test", HttpResponse::ok()),
    ]))
    .expect("start mock registry");

    // ACT
    let error = match imager::sign(&registry.reference("repo", "test"), &keys.private_key_pem).await
    {
        Ok(()) => {
            panic!("sign unexpectedly succeeded");
        }
        Err(error) => error,
    };

    // ASSERT
    assert!(matches!(error, ImagerError::InvalidOciFormat(_)));
}

#[test]
fn cli_pull_extracts_signed_image_with_pub_key() {
    // ARRANGE
    let keys = generate_test_keys().expect("generate test keys");
    let workspace = TempDir::new().expect("create temp dir");
    let output_dir = workspace.path().join("out");
    let pub_key_path = workspace.path().join("imager.pub.pem");
    std::fs::write(&pub_key_path, &keys.public_key_pem).expect("write public key");

    let layer = layer_archive(&[("etc/cli", b"pulled from cli\n")]).expect("build layer archive");
    let layer_digest = sha256_digest(&layer);
    let manifest = signed_manifest_json(
        &manifest_json(&layer_digest, layer.len()).expect("build manifest json"),
        &keys.private_key_pem,
    )
    .expect("sign manifest json");
    let registry = MockRegistry::start(std::collections::HashMap::from([
        get("/v2/repo/manifests/test", HttpResponse::json(manifest)),
        get(
            format!("/v2/repo/blobs/{layer_digest}"),
            HttpResponse::octet_stream(layer),
        ),
    ]))
    .expect("start mock registry");

    // ACT
    let output = Command::new(imager_bin())
        .arg("pull")
        .arg("--image")
        .arg(registry.reference("repo", "test"))
        .arg("--output")
        .arg(&output_dir)
        .arg("--pub-key")
        .arg(&pub_key_path)
        .output()
        .expect("run imager pull");

    // ASSERT
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Successfully extracted image to"),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        std::fs::read_to_string(output_dir.join("etc/cli")).expect("read extracted file"),
        "pulled from cli\n"
    );
}

#[test]
fn cli_sign_signs_manifest_in_registry() {
    // ARRANGE
    let keys = generate_test_keys().expect("generate test keys");
    let workspace = TempDir::new().expect("create temp dir");
    let key_path = workspace.path().join("imager.key.pem");
    std::fs::write(&key_path, &keys.private_key_pem).expect("write private key");

    let path = "/v2/repo/manifests/test";
    let registry = MockRegistry::start(std::collections::HashMap::from([
        get(
            path,
            HttpResponse::json(minimal_manifest_json().expect("build manifest json")),
        ),
        put(path, HttpResponse::ok()),
    ]))
    .expect("start mock registry");

    // ACT
    let output = Command::new(imager_bin())
        .arg("sign")
        .arg("--image")
        .arg(registry.reference("repo", "test"))
        .arg("--key")
        .arg(&key_path)
        .output()
        .expect("run imager sign");

    // ASSERT
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Successfully signed"),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    let request = registry
        .request("PUT", path)
        .expect("read request log")
        .unwrap_or_else(|| panic!("missing signed manifest PUT request"));
    let manifest: Value =
        serde_json::from_slice(&request.body).expect("parse signed manifest body");
    assert!(manifest["annotations"]["dev.muak.sig"].as_str().is_some());
}

#[test]
fn cli_reports_key_file_read_error() {
    // ARRANGE
    let workspace = TempDir::new().expect("create temp dir");
    let missing_path = workspace.path().join("missing.pem");

    // ACT
    let output = Command::new(imager_bin())
        .arg("sign")
        .arg("--image")
        .arg("127.0.0.1:9/repo:test")
        .arg("--key")
        .arg(&missing_path)
        .output()
        .expect("run imager sign");

    // ASSERT
    assert!(!output.status.success(), "sign unexpectedly succeeded");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Failed to read key from"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
