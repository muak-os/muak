//! Integration tests for koci OCI pulling and signing.

mod fixtures;
mod registry;

use std::path::Path;
use std::process::Command;

use fixtures::*;
use koci::error::KociError;
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

fn koci_bin() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_koci"))
}

#[tokio::test]
async fn pull_extracts_single_layer_from_local_registry() {
    // ARRANGE
    let layer = layer_archive(&[
        ("etc/motd", b"hello from koci\n"),
        ("usr/share/koci/message.txt", b"integration test\n"),
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
    koci::pull(&registry.reference("repo", "test"), output.path(), None)
        .await
        .expect("pull image");

    // ASSERT
    assert_eq!(
        std::fs::read_to_string(output.path().join("etc/motd")).expect("read motd"),
        "hello from koci\n"
    );
    assert_eq!(
        std::fs::read_to_string(output.path().join("usr/share/koci/message.txt"))
            .expect("read message file"),
        "integration test\n"
    );
}

#[tokio::test]
async fn pull_selects_requested_platform_manifest_from_index() {
    // ARRANGE
    let layer = layer_archive(&[("etc/platform", b"selected requested manifest\n")])
        .expect("build layer archive");
    let layer_digest = sha256_digest(&layer);
    let selected_manifest_digest =
        "sha256:2222222222222222222222222222222222222222222222222222222222222222";
    let fallback_manifest_digest =
        "sha256:3333333333333333333333333333333333333333333333333333333333333333";

    let index = index_for_arches_json(&[
        (fallback_manifest_digest, "amd64", "linux"),
        (selected_manifest_digest, "arm64", "linux"),
    ])
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
    koci::pull_arch(
        &registry.reference("repo", "test"),
        "arm64",
        output.path(),
        None,
    )
    .await
    .expect("pull image");

    // ASSERT
    assert_eq!(
        std::fs::read_to_string(output.path().join("etc/platform")).expect("read platform file"),
        "selected requested manifest\n"
    );
}

#[tokio::test]
async fn pull_rejects_index_without_requested_platform_match() {
    // ARRANGE
    let index = index_for_arches_json(&[(
        "sha256:3333333333333333333333333333333333333333333333333333333333333333",
        "amd64",
        "windows",
    )])
    .expect("build index json");
    let registry = MockRegistry::start(std::collections::HashMap::from([get(
        "/v2/repo/manifests/test",
        HttpResponse::index(index),
    )]))
    .expect("start mock registry");
    let output = TempDir::new().expect("create temp dir");

    // ACT
    let error = koci::pull_arch(
        &registry.reference("repo", "test"),
        "arm64",
        output.path(),
        None,
    )
    .await
    .expect_err("pull should fail");

    // ASSERT
    assert!(matches!(error, KociError::InvalidOciFormat(_)));
}

#[tokio::test]
async fn pull_supports_digest_manifest_reference() {
    // ARRANGE
    let layer = layer_archive(&[("etc/digest", b"pulled by digest\n")]).expect("build layer");
    let layer_digest = sha256_digest(&layer);
    let manifest = manifest_json(&layer_digest, layer.len()).expect("build manifest json");
    let manifest_digest = sha256_digest(&manifest);

    let registry = MockRegistry::start(std::collections::HashMap::from([
        get(
            format!("/v2/repo/manifests/{manifest_digest}"),
            HttpResponse::json(manifest),
        ),
        get(
            format!("/v2/repo/blobs/{layer_digest}"),
            HttpResponse::octet_stream(layer),
        ),
    ]))
    .expect("start mock registry");
    let output = TempDir::new().expect("create temp dir");

    // ACT
    koci::pull(
        &registry.digest_reference("repo", &manifest_digest),
        output.path(),
        None,
    )
    .await
    .expect("pull image by digest");

    // ASSERT
    assert_eq!(
        std::fs::read_to_string(output.path().join("etc/digest")).expect("read extracted file"),
        "pulled by digest\n"
    );
    let request = required_request(
        &registry,
        "GET",
        &format!("/v2/repo/manifests/{manifest_digest}"),
    );
    assert_eq!(
        request.path,
        format!("/v2/repo/manifests/{manifest_digest}")
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
    let error = match koci::pull(&registry.reference("repo", "test"), output.path(), None).await {
        Ok(()) => {
            panic!("pull unexpectedly succeeded");
        }
        Err(error) => error,
    };

    // ASSERT
    assert!(matches!(error, KociError::DigestMismatch { .. }));
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
    koci::sign(&registry.reference("repo", "test"), &keys.private_key_pem)
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
    koci::sign(&registry.reference("repo", "test"), &keys.private_key_pem)
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
async fn sign_signs_single_manifest_in_registry() {
    // ARRANGE
    let keys = generate_test_keys().expect("generate test keys");
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
    koci::sign(&registry.reference("repo", "test"), &keys.private_key_pem)
        .await
        .expect("sign image");

    // ASSERT
    let request = required_request(&registry, "PUT", path);
    let manifest: Value =
        serde_json::from_slice(&request.body).expect("parse signed manifest body");
    assert!(manifest["annotations"]["dev.muak.sig"].as_str().is_some());
}

#[tokio::test]
async fn sign_rejects_invalid_private_key_before_network() {
    // ARRANGE / ACT
    let error = match koci::sign("127.0.0.1:9/repo:test", "not a pem file").await {
        Ok(()) => {
            panic!("sign unexpectedly succeeded");
        }
        Err(error) => error,
    };

    // ASSERT
    assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
}

#[tokio::test]
async fn sign_rejects_invalid_private_key_base64_before_network() {
    // ARRANGE
    let private_key_pem = "-----BEGIN PRIVATE KEY-----\n!!!\n-----END PRIVATE KEY-----\n";

    // ACT
    let error = match koci::sign("127.0.0.1:9/repo:test", private_key_pem).await {
        Ok(()) => {
            panic!("sign unexpectedly succeeded");
        }
        Err(error) => error,
    };

    // ASSERT
    assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    assert!(
        error
            .to_string()
            .contains("Failed to decode private key from PEM")
    );
}

#[tokio::test]
async fn sign_rejects_invalid_private_key_pkcs8_before_network() {
    // ARRANGE
    let private_key_pem = "-----BEGIN PRIVATE KEY-----\nAAECAwQFBgc=\n-----END PRIVATE KEY-----\n";

    // ACT
    let error = match koci::sign("127.0.0.1:9/repo:test", private_key_pem).await {
        Ok(()) => {
            panic!("sign unexpectedly succeeded");
        }
        Err(error) => error,
    };

    // ASSERT
    assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    assert!(
        error
            .to_string()
            .contains("Failed to parse ECDSA P-256 private key")
    );
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
    let error = match koci::sign(&registry.reference("repo", "test"), &keys.private_key_pem).await {
        Ok(()) => {
            panic!("sign unexpectedly succeeded");
        }
        Err(error) => error,
    };

    // ASSERT
    assert!(matches!(error, KociError::InvalidOciFormat(_)));
}

#[test]
fn cli_pull_extracts_signed_image_with_pub_key() {
    // ARRANGE
    let keys = generate_test_keys().expect("generate test keys");
    let workspace = TempDir::new().expect("create temp dir");
    let output_dir = workspace.path().join("out");
    let pub_key_path = workspace.path().join("koci.pub.pem");
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
    let output = Command::new(koci_bin())
        .arg("pull")
        .arg("--image")
        .arg(registry.reference("repo", "test"))
        .arg("--output")
        .arg(&output_dir)
        .arg("--pub-key")
        .arg(&pub_key_path)
        .output()
        .expect("run koci pull");

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
fn cli_pull_reports_missing_pubkey_file() {
    // ARRANGE
    let workspace = TempDir::new().expect("create temp dir");
    let output_dir = workspace.path().join("out");
    let missing_key = workspace.path().join("missing.pub.pem");

    // ACT
    let output = Command::new(koci_bin())
        .arg("pull")
        .arg("--image")
        .arg("127.0.0.1:9/repo:test")
        .arg("--output")
        .arg(&output_dir)
        .arg("--pub-key")
        .arg(&missing_key)
        .output()
        .expect("run koci pull");

    // ASSERT
    assert!(!output.status.success(), "pull unexpectedly succeeded");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Failed to read key from"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cli_pull_uses_explicit_arch_argument() {
    // ARRANGE
    let workspace = TempDir::new().expect("create temp dir");
    let output_dir = workspace.path().join("out");
    let selected_layer = layer_archive(&[("etc/arch", b"arm64\n")]).expect("build layer archive");
    let selected_layer_digest = sha256_digest(&selected_layer);
    let selected_manifest_digest =
        "sha256:2222222222222222222222222222222222222222222222222222222222222222";
    let other_manifest_digest =
        "sha256:3333333333333333333333333333333333333333333333333333333333333333";
    let index = index_for_arches_json(&[
        (other_manifest_digest, "amd64", "linux"),
        (selected_manifest_digest, "arm64", "linux"),
    ])
    .expect("build index json");
    let selected_manifest =
        manifest_json(&selected_layer_digest, selected_layer.len()).expect("build manifest json");
    let other_manifest = minimal_manifest_json().expect("build fallback manifest json");
    let registry = MockRegistry::start(std::collections::HashMap::from([
        get("/v2/repo/manifests/test", HttpResponse::index(index)),
        get(
            format!("/v2/repo/manifests/{selected_manifest_digest}"),
            HttpResponse::json(selected_manifest),
        ),
        get(
            format!("/v2/repo/manifests/{other_manifest_digest}"),
            HttpResponse::json(other_manifest),
        ),
        get(
            format!("/v2/repo/blobs/{selected_layer_digest}"),
            HttpResponse::octet_stream(selected_layer),
        ),
    ]))
    .expect("start mock registry");

    // ACT
    let output = Command::new(koci_bin())
        .arg("pull")
        .arg("--image")
        .arg(registry.reference("repo", "test"))
        .arg("--arch")
        .arg("arm64")
        .arg("--output")
        .arg(&output_dir)
        .output()
        .expect("run koci pull");

    // ASSERT
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(output_dir.join("etc/arch")).expect("read extracted file"),
        "arm64\n"
    );
}

#[tokio::test]
async fn pull_applies_multiple_layers_in_order() {
    // ARRANGE
    let first_layer = layer_archive(&[("etc/message", b"first\n")]).expect("build first layer");
    let second_layer = layer_archive(&[("etc/.wh.message", b""), ("etc/message", b"second\n")])
        .expect("build second layer");
    let first_digest = sha256_digest(&first_layer);
    let second_digest = sha256_digest(&second_layer);
    let manifest = manifest_with_layers_json(&[
        (
            &first_digest,
            first_layer.len(),
            "application/vnd.oci.image.layer.v1.tar+gzip",
        ),
        (
            &second_digest,
            second_layer.len(),
            "application/vnd.oci.image.layer.v1.tar+gzip",
        ),
    ])
    .expect("build manifest json");
    let registry = MockRegistry::start(std::collections::HashMap::from([
        get("/v2/repo/manifests/test", HttpResponse::json(manifest)),
        get(
            format!("/v2/repo/blobs/{first_digest}"),
            HttpResponse::octet_stream(first_layer),
        ),
        get(
            format!("/v2/repo/blobs/{second_digest}"),
            HttpResponse::octet_stream(second_layer),
        ),
    ]))
    .expect("start mock registry");
    let output = TempDir::new().expect("create temp dir");

    // ACT
    koci::pull(&registry.reference("repo", "test"), output.path(), None)
        .await
        .expect("pull image");

    // ASSERT
    assert_eq!(
        std::fs::read_to_string(output.path().join("etc/message")).expect("read extracted file"),
        "second\n"
    );
}

#[tokio::test]
async fn pull_rejects_non_utf8_manifest_response() {
    // ARRANGE
    let registry = MockRegistry::start(std::collections::HashMap::from([get(
        "/v2/repo/manifests/test",
        HttpResponse::json(vec![0xff, 0xfe, 0xfd]),
    )]))
    .expect("start mock registry");
    let output = TempDir::new().expect("create temp dir");

    // ACT
    let error = koci::pull(&registry.reference("repo", "test"), output.path(), None)
        .await
        .expect_err("pull should fail");

    // ASSERT
    assert!(matches!(error, KociError::NetworkError(_)));
}

#[tokio::test]
async fn pull_rejects_invalid_manifest_json() {
    // ARRANGE
    let registry = MockRegistry::start(std::collections::HashMap::from([get(
        "/v2/repo/manifests/test",
        HttpResponse::json(b"not json".to_vec()),
    )]))
    .expect("start mock registry");
    let output = TempDir::new().expect("create temp dir");

    // ACT
    let error = koci::pull(&registry.reference("repo", "test"), output.path(), None)
        .await
        .expect_err("pull should fail");

    // ASSERT
    assert!(matches!(error, KociError::OciParseError(_)));
}

#[test]
fn cli_sign_signs_manifest_in_registry() {
    // ARRANGE
    let keys = generate_test_keys().expect("generate test keys");
    let workspace = TempDir::new().expect("create temp dir");
    let key_path = workspace.path().join("koci.key.pem");
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
    let output = Command::new(koci_bin())
        .arg("sign")
        .arg("--image")
        .arg(registry.reference("repo", "test"))
        .arg("--key")
        .arg(&key_path)
        .output()
        .expect("run koci sign");

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
    let output = Command::new(koci_bin())
        .arg("sign")
        .arg("--image")
        .arg("127.0.0.1:9/repo:test")
        .arg("--key")
        .arg(&missing_path)
        .output()
        .expect("run koci sign");

    // ASSERT
    assert!(!output.status.success(), "sign unexpectedly succeeded");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Failed to read key from"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
