//! Integration tests for koci OCI pulling and signing.

extern crate alloc;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod registry;

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;
    use std::process::Command;
    use std::time::{Duration, Instant, SystemTime};

    use koci::annotations;
    use koci::arch::Arch;
    use koci::error::KociError;
    use koci::merge;
    use koci::pull;
    use serde_json::Value;
    use tempfile::TempDir;

    use super::fixtures::*;
    use super::registry::{HttpResponse, MockRegistry, RecordedRequest, get, put};

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

    struct CollectedFile {
        path: String,
        contents: Vec<u8>,
    }

    fn collect_files(reference: &str, arch: Arch) -> Vec<CollectedFile> {
        let mut files = Vec::new();
        pull::files(reference, &arch, None, |entry| {
            let path = entry.path.clone();
            let mut contents = Vec::new();
            entry.reader.read_to_end(&mut contents)?;
            files.push(CollectedFile { path, contents });
            Ok(())
        })
        .expect("stream files should succeed");
        files
    }

    fn expect_stream_error(reference: &str) -> KociError {
        pull::files(reference, &Arch::Amd64, None, |_entry| Ok(())).expect_err("stream should fail")
    }

    fn expect_sign_error(reference: &str, private_key_pem: &str) -> KociError {
        annotations::sign(reference, private_key_pem, SIG_ANNOTATION).expect_err("sign should fail")
    }

    fn assert_signed_manifest_has_signature(registry: &MockRegistry, path: &str) {
        let request = required_request(registry, "PUT", path);
        let manifest: Value =
            serde_json::from_slice(&request.body).expect("parse signed manifest body");
        let signature = manifest
            .get("annotations")
            .and_then(|annotations| annotations.get("dev.muak.sig"))
            .and_then(Value::as_str);
        assert!(
            signature.is_some(),
            "signed manifest must include the signature annotation"
        );
    }

    fn assert_signed_manifests_have_signatures(registry: &MockRegistry, paths: &[String]) {
        for path in paths {
            assert_signed_manifest_has_signature(registry, path);
        }
    }

    #[test]
    fn stream_files_yields_files_from_single_layer() {
        // ARRANGE
        let layer = layer_archive(&[
            ("etc/motd", b"hello from koci\n"),
            ("usr/share/koci/message.txt", b"integration test\n"),
        ])
        .expect("build layer archive");
        let layer_digest = sha256_digest(&layer);
        let manifest = manifest_json(&layer_digest, layer.len()).expect("build manifest json");

        let registry = MockRegistry::start(HashMap::from([
            get("/v2/repo/manifests/test", HttpResponse::json(manifest)),
            get(
                format!("/v2/repo/blobs/{layer_digest}"),
                HttpResponse::octet_stream(layer),
            ),
        ]))
        .expect("start mock registry");

        // ACT
        let files = collect_files(&registry.reference("repo", "test"), Arch::Amd64);

        // ASSERT
        assert_eq!(files.len(), 2);
        assert_eq!(files.first().unwrap().path, "etc/motd");
        assert_eq!(&files.first().unwrap().contents, b"hello from koci\n");
        assert_eq!(files.get(1).unwrap().path, "usr/share/koci/message.txt");
        assert_eq!(&files.get(1).unwrap().contents, b"integration test\n");
    }

    #[test]
    fn stream_files_selects_requested_platform() {
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
        let selected_manifest =
            manifest_json(&layer_digest, layer.len()).expect("build manifest json");
        let fallback_manifest = manifest_json(
            "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            1,
        )
        .expect("build fallback manifest json");

        let registry = MockRegistry::start(HashMap::from([
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

        // ACT
        let files = collect_files(&registry.reference("repo", "test"), Arch::Arm64);

        // ASSERT
        assert_eq!(files.len(), 1);
        assert_eq!(
            &files.first().unwrap().contents,
            b"selected requested manifest\n"
        );
    }

    #[test]
    fn stream_files_rejects_missing_platform() {
        // ARRANGE
        let index = index_for_arches_json(&[(
            "sha256:3333333333333333333333333333333333333333333333333333333333333333",
            "amd64",
            "windows",
        )])
        .expect("build index json");
        let registry = MockRegistry::start(HashMap::from([get(
            "/v2/repo/manifests/test",
            HttpResponse::index(index),
        )]))
        .expect("start mock registry");

        // ACT
        let error = pull::files(
            &registry.reference("repo", "test"),
            &Arch::Arm64,
            None,
            |_entry| Ok(()),
        )
        .expect_err("stream should fail");

        // ASSERT
        assert!(matches!(error, KociError::InvalidOciFormat(_)));
    }

    #[test]
    fn stream_files_supports_digest_reference() {
        // ARRANGE
        let layer = layer_archive(&[("etc/digest", b"pulled by digest\n")]).expect("build layer");
        let layer_digest = sha256_digest(&layer);
        let manifest = manifest_json(&layer_digest, layer.len()).expect("build manifest json");
        let manifest_digest = sha256_digest(&manifest);

        let registry = MockRegistry::start(HashMap::from([
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

        // ACT
        let files = collect_files(
            &registry.digest_reference("repo", &manifest_digest),
            Arch::Amd64,
        );

        // ASSERT
        assert_eq!(files.len(), 1);
        assert_eq!(&files.first().unwrap().contents, b"pulled by digest\n");
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

    #[test]
    fn stream_files_rejects_blob_digest_mismatch() {
        // ARRANGE
        let layer =
            layer_archive(&[("etc/invalid", b"digest mismatch\n")]).expect("build layer archive");
        let manifest = manifest_json(
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            layer.len(),
        )
        .expect("build manifest json");

        let registry = MockRegistry::start(HashMap::from([
            get("/v2/repo/manifests/test", HttpResponse::json(manifest)),
            get(
                "/v2/repo/blobs/sha256:0000000000000000000000000000000000000000000000000000000000000000",
                HttpResponse::octet_stream(layer),
            ),
        ]))
        .expect("start mock registry");

        // ACT
        let error = expect_stream_error(&registry.reference("repo", "test"));

        // ASSERT
        assert!(matches!(error, KociError::DigestMismatch { .. }));
    }

    #[test]
    fn sign_signs_index_and_platform_manifests() {
        // ARRANGE
        let keys = generate_test_keys().expect("generate test keys");
        let top_level_path = "/v2/repo/manifests/test";
        let first_platform_digest =
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let second_platform_digest =
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

        let registry = MockRegistry::start(HashMap::from([
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
        annotations::sign(
            &registry.reference("repo", "test"),
            &keys.private_key_pem,
            SIG_ANNOTATION,
        )
        .expect("sign image");

        // ASSERT
        let signed_paths = [
            top_level_path.to_owned(),
            format!("/v2/repo/manifests/{first_platform_digest}"),
            format!("/v2/repo/manifests/{second_platform_digest}"),
        ];
        assert_signed_manifests_have_signatures(&registry, &signed_paths);
    }

    #[test]
    fn sign_uses_default_manifest_content_type_when_media_type_is_missing() {
        // ARRANGE
        let keys = generate_test_keys().expect("generate test keys");
        let path = "/v2/repo/manifests/test";
        let registry = MockRegistry::start(HashMap::from([
            get(
                path,
                HttpResponse::json(
                    manifest_without_media_type_json().expect("build manifest json"),
                ),
            ),
            put(path, HttpResponse::ok()),
        ]))
        .expect("start mock registry");

        // ACT
        annotations::sign(
            &registry.reference("repo", "test"),
            &keys.private_key_pem,
            SIG_ANNOTATION,
        )
        .expect("sign image");

        // ASSERT
        let request = required_request(&registry, "PUT", path);

        assert_eq!(
            request.headers.get("content-type").map(String::as_str),
            Some("application/vnd.oci.image.manifest.v1+json")
        );
    }

    #[test]
    fn sign_signs_single_manifest_in_registry() {
        // ARRANGE
        let keys = generate_test_keys().expect("generate test keys");
        let path = "/v2/repo/manifests/test";
        let registry = MockRegistry::start(HashMap::from([
            get(
                path,
                HttpResponse::json(minimal_manifest_json().expect("build manifest json")),
            ),
            put(path, HttpResponse::ok()),
        ]))
        .expect("start mock registry");

        // ACT
        annotations::sign(
            &registry.reference("repo", "test"),
            &keys.private_key_pem,
            SIG_ANNOTATION,
        )
        .expect("sign image");

        // ASSERT
        let request = required_request(&registry, "PUT", path);
        let manifest: Value =
            serde_json::from_slice(&request.body).expect("parse signed manifest body");
        assert!(
            manifest
                .get("annotations")
                .and_then(|annotations| annotations.get("dev.muak.sig"))
                .and_then(Value::as_str)
                .is_some()
        );
    }

    #[test]
    fn merge_creates_multi_arch_index_from_per_arch_manifests() {
        // ARRANGE
        let amd64_manifest = manifest_json("sha256:aaa", 1).expect("build amd64 manifest");
        let arm64_manifest = manifest_json("sha256:bbb", 2).expect("build arm64 manifest");
        let registry = MockRegistry::start(HashMap::from([
            get(
                "/v2/repo/manifests/v1-amd64",
                HttpResponse::json(amd64_manifest.clone()),
            ),
            get(
                "/v2/repo/manifests/v1-arm64",
                HttpResponse::json(arm64_manifest.clone()),
            ),
            put("/v2/repo/manifests/v1", HttpResponse::ok()),
            put("/v2/repo/manifests/latest", HttpResponse::ok()),
        ]))
        .expect("start mock registry");
        let sources = vec![
            merge::Source {
                arch: Arch::Amd64,
                reference: "v1-amd64".to_owned(),
            },
            merge::Source {
                arch: Arch::Arm64,
                reference: "v1-arm64".to_owned(),
            },
        ];

        // ACT
        merge::index(
            &registry.reference("repo", "v1"),
            &["v1".to_owned(), "latest".to_owned()],
            &sources,
        )
        .expect("merge should succeed");

        // ASSERT
        for tag in ["v1", "latest"] {
            let request = required_request(&registry, "PUT", &format!("/v2/repo/manifests/{tag}"));
            let index: Value = serde_json::from_slice(&request.body).expect("parse index body");
            assert_eq!(index.get("schemaVersion").and_then(Value::as_u64), Some(2));
            assert_eq!(
                index.get("mediaType").and_then(Value::as_str),
                Some("application/vnd.oci.image.index.v1+json")
            );

            let manifests = index
                .get("manifests")
                .and_then(Value::as_array)
                .expect("manifests array");
            assert_eq!(manifests.len(), 2);

            let amd64_descriptor = manifests.first().expect("amd64 descriptor");
            assert_eq!(
                amd64_descriptor
                    .get("platform")
                    .and_then(|platform| platform.get("architecture"))
                    .and_then(Value::as_str),
                Some("amd64")
            );
            assert_eq!(
                amd64_descriptor.get("digest").and_then(Value::as_str),
                Some(sha256_digest(&amd64_manifest).as_str()),
            );
            assert_eq!(
                amd64_descriptor.get("size").and_then(Value::as_u64),
                Some(u64::try_from(amd64_manifest.len()).expect("size fits u64"))
            );

            let arm64_descriptor = manifests.get(1).expect("arm64 descriptor");
            assert_eq!(
                arm64_descriptor
                    .get("platform")
                    .and_then(|platform| platform.get("architecture"))
                    .and_then(Value::as_str),
                Some("arm64")
            );
            assert_eq!(
                arm64_descriptor.get("digest").and_then(Value::as_str),
                Some(sha256_digest(&arm64_manifest).as_str()),
            );
            assert_eq!(
                arm64_descriptor.get("size").and_then(Value::as_u64),
                Some(u64::try_from(arm64_manifest.len()).expect("size fits u64"))
            );
        }
    }

    #[test]
    fn merge_rejects_duplicate_platforms_before_network() {
        // ARRANGE
        let sources = vec![
            merge::Source {
                arch: Arch::Amd64,
                reference: "v1-amd64".to_owned(),
            },
            merge::Source {
                arch: Arch::Amd64,
                reference: "v1-amd64-copy".to_owned(),
            },
        ];

        // ACT
        let error = merge::index("127.0.0.1:9/repo:v1", &["v1".to_owned()], &sources)
            .expect_err("duplicate platforms should fail");

        // ASSERT
        assert!(matches!(error, KociError::InvalidOciFormat(_)));
    }

    #[test]
    fn sign_rejects_invalid_private_key_before_network() {
        // ARRANGE / ACT
        let error = expect_sign_error("127.0.0.1:9/repo:test", "not a pem file");

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }

    #[test]
    fn sign_rejects_invalid_private_key_base64_before_network() {
        // ARRANGE
        let private_key_pem = "-----BEGIN PRIVATE KEY-----\n!!!\n-----END PRIVATE KEY-----\n";

        // ACT
        let error = expect_sign_error("127.0.0.1:9/repo:test", private_key_pem);

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
        assert!(
            error
                .to_string()
                .contains("Failed to parse ECDSA P-256 private key")
        );
    }

    #[test]
    fn sign_rejects_invalid_private_key_pkcs8_before_network() {
        // ARRANGE
        let private_key_pem =
            "-----BEGIN PRIVATE KEY-----\nAAECAwQFBgc=\n-----END PRIVATE KEY-----\n";

        // ACT
        let error = expect_sign_error("127.0.0.1:9/repo:test", private_key_pem);

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
        assert!(
            error
                .to_string()
                .contains("Failed to parse ECDSA P-256 private key")
        );
    }

    #[test]
    fn sign_rejects_non_object_manifest_annotations() {
        // ARRANGE
        let keys = generate_test_keys().expect("generate test keys");
        let registry = MockRegistry::start(HashMap::from([
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
        let error = expect_sign_error(&registry.reference("repo", "test"), &keys.private_key_pem);

        // ASSERT
        assert!(matches!(error, KociError::OciParseError(_)));
    }

    #[test]
    fn cli_pull_extracts_signed_image_with_pub_key() {
        // ARRANGE
        let keys = generate_test_keys().expect("generate test keys");
        let workspace = TempDir::new().expect("create temp dir");
        let output_dir = workspace.path().join("out");
        let key_path = workspace.path().join("koci.key.pem");
        std::fs::write(&key_path, &keys.private_key_pem).expect("write private key");
        let pub_key_path = workspace.path().join("koci.pub.pem");
        std::fs::write(&pub_key_path, &keys.public_key_pem).expect("write public key");

        let layer =
            layer_archive(&[("etc/cli", b"pulled from cli\n")]).expect("build layer archive");
        let layer_digest = sha256_digest(&layer);
        let manifest = manifest_json(&layer_digest, layer.len()).expect("build manifest json");
        let registry = MockRegistry::start(HashMap::from([
            get("/v2/repo/manifests/test", HttpResponse::json(manifest)),
            get(
                format!("/v2/repo/blobs/{layer_digest}"),
                HttpResponse::octet_stream(layer),
            ),
            put("/v2/repo/manifests/test", HttpResponse::ok()),
        ]))
        .expect("start mock registry");

        annotations::sign(
            &registry.reference("repo", "test"),
            &keys.private_key_pem,
            SIG_ANNOTATION,
        )
        .expect("sign image");

        // ACT
        let output = Command::new(koci_bin())
            .arg("pull")
            .arg("--image")
            .arg(registry.reference("repo", "test"))
            .arg("--output")
            .arg(&output_dir)
            .arg("--pub-key")
            .arg(&pub_key_path)
            .arg("--sig-annotation")
            .arg("dev.muak.sig")
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
            .arg("--sig-annotation")
            .arg("dev.muak.sig")
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
        let selected_layer =
            layer_archive(&[("etc/arch", b"arm64\n")]).expect("build layer archive");
        let selected_layer_digest = sha256_digest(&selected_layer);
        let selected_manifest_digest =
            "sha256:4444444444444444444444444444444444444444444444444444444444444444";
        let other_manifest_digest =
            "sha256:5555555555555555555555555555555555555555555555555555555555555555";
        let index = index_for_arches_json(&[
            (other_manifest_digest, "amd64", "linux"),
            (selected_manifest_digest, "arm64", "linux"),
        ])
        .expect("build index json");
        let selected_manifest = manifest_json(&selected_layer_digest, selected_layer.len())
            .expect("build manifest json");
        let other_manifest = minimal_manifest_json().expect("build fallback manifest json");
        let registry = MockRegistry::start(HashMap::from([
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

    #[test]
    fn stream_files_applies_multiple_layers_with_whiteouts() {
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
        let registry = MockRegistry::start(HashMap::from([
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

        // ACT
        let files = collect_files(&registry.reference("repo", "test"), Arch::Amd64);

        // ASSERT
        assert_eq!(files.len(), 1);
        assert_eq!(files.first().unwrap().path, "etc/message");
        assert_eq!(&files.first().unwrap().contents, b"second\n");
    }

    #[test]
    fn stream_files_downloads_layers_in_parallel() {
        // ARRANGE
        let marker = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
            .to_string();
        let first = format!("first-{marker}\n");
        let second = format!("second-{marker}\n");
        let first_layer =
            layer_archive(&[("etc/first", first.as_bytes())]).expect("build first layer");
        let second_layer =
            layer_archive(&[("etc/second", second.as_bytes())]).expect("build second layer");
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
        let delay = Duration::from_millis(500);
        let registry = MockRegistry::start(HashMap::from([
            get("/v2/repo/manifests/test", HttpResponse::json(manifest)),
            get(
                format!("/v2/repo/blobs/{first_digest}"),
                HttpResponse::octet_stream(first_layer).with_delay(delay),
            ),
            get(
                format!("/v2/repo/blobs/{second_digest}"),
                HttpResponse::octet_stream(second_layer).with_delay(delay),
            ),
        ]))
        .expect("start mock registry");

        // ACT
        let start = Instant::now();
        collect_files(&registry.reference("repo", "test"), Arch::Amd64);
        let elapsed = start.elapsed();

        // ASSERT
        assert!(
            elapsed < Duration::from_millis(900),
            "parallel layer downloads should take about {delay:?}, took {elapsed:?}"
        );
    }

    #[test]
    fn stream_files_rejects_unsupported_layer_media_type() {
        // ARRANGE
        let layer = b"plain bytes".to_vec();
        let layer_digest = sha256_digest(&layer);
        let manifest =
            manifest_with_layers_json(&[(&layer_digest, layer.len(), "application/test")])
                .expect("build manifest json");
        let registry = MockRegistry::start(HashMap::from([
            get("/v2/repo/manifests/test", HttpResponse::json(manifest)),
            get(
                format!("/v2/repo/blobs/{layer_digest}"),
                HttpResponse::octet_stream(layer),
            ),
        ]))
        .expect("start mock registry");

        // ACT
        let error = expect_stream_error(&registry.reference("repo", "test"));

        // ASSERT
        assert!(matches!(error, KociError::UnsupportedLayerMediaType(_)));
    }

    #[test]
    fn stream_files_allows_empty_layer_list() {
        // ARRANGE
        let registry = MockRegistry::start(HashMap::from([get(
            "/v2/repo/manifests/test",
            HttpResponse::json(minimal_manifest_json().expect("build manifest json")),
        )]))
        .expect("start mock registry");

        // ACT
        let files = collect_files(&registry.reference("repo", "test"), Arch::Amd64);

        // ASSERT
        assert!(files.is_empty());
    }

    #[test]
    fn stream_files_rejects_missing_blob() {
        // ARRANGE
        let layer_digest =
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let manifest = manifest_json(layer_digest, 1).expect("build manifest json");
        let registry = MockRegistry::start(HashMap::from([get(
            "/v2/repo/manifests/test",
            HttpResponse::json(manifest),
        )]))
        .expect("start mock registry");

        // ACT
        let error = expect_stream_error(&registry.reference("repo", "test"));

        // ASSERT
        assert!(matches!(error, KociError::DownloadError(_)));
    }

    #[test]
    fn annotations_return_all_annotations_of_the_selected_manifest() {
        // ARRANGE
        let layer = layer_archive(&[("etc/motd", b"hello\n")]).expect("build layer archive");
        let layer_digest = sha256_digest(&layer);
        let manifest = annotated_manifest_json(
            &layer_digest,
            layer.len(),
            &[
                (SIG_ANNOTATION, "AA"),
                (SIZES_ANNOTATION, r#"{"etc/motd":6}"#),
            ],
        )
        .expect("build annotated manifest json");

        let registry = MockRegistry::start(HashMap::from([get(
            "/v2/repo/manifests/test",
            HttpResponse::json(manifest),
        )]))
        .expect("start mock registry");

        // ACT
        let annotations =
            pull::annotations(&registry.reference("repo", "test"), &Arch::Amd64, None)
                .expect("read annotations");

        // ASSERT
        assert_eq!(
            annotations.get("dev.muak.sig").map(String::as_str),
            Some("AA")
        );
        assert_eq!(
            annotations.get("dev.muak.sizes").map(String::as_str),
            Some(r#"{"etc/motd":6}"#),
        );
    }

    #[test]
    fn annotations_select_the_requested_platform_manifest() {
        // ARRANGE
        let amd64_digest =
            "sha256:1111111111111111111111111111111111111111111111111111111111111111";
        let arm64_digest =
            "sha256:2222222222222222222222222222222222222222222222222222222222222222";
        let index = index_for_arches_json(&[
            (amd64_digest, "amd64", "linux"),
            (arm64_digest, "arm64", "linux"),
        ])
        .expect("build index json");
        let amd64_manifest = annotated_manifest_json(
            amd64_digest,
            1,
            &[(SIZES_ANNOTATION, r#"{"vmlinuz-amd64":1}"#)],
        )
        .expect("build amd64 manifest json");
        let arm64_manifest = annotated_manifest_json(
            arm64_digest,
            2,
            &[(SIZES_ANNOTATION, r#"{"vmlinuz-arm64":2}"#)],
        )
        .expect("build arm64 manifest json");

        let registry = MockRegistry::start(HashMap::from([
            get("/v2/repo/manifests/test", HttpResponse::index(index)),
            get(
                format!("/v2/repo/manifests/{amd64_digest}"),
                HttpResponse::json(amd64_manifest),
            ),
            get(
                format!("/v2/repo/manifests/{arm64_digest}"),
                HttpResponse::json(arm64_manifest),
            ),
        ]))
        .expect("start mock registry");

        // ACT
        let annotations =
            pull::annotations(&registry.reference("repo", "test"), &Arch::Arm64, None)
                .expect("read annotations");

        // ASSERT
        assert_eq!(
            annotations.get("dev.muak.sizes").map(String::as_str),
            Some(r#"{"vmlinuz-arm64":2}"#),
        );
    }

    #[test]
    fn annotations_are_empty_when_the_manifest_carries_none() {
        // ARRANGE
        let registry = MockRegistry::start(HashMap::from([get(
            "/v2/repo/manifests/test",
            HttpResponse::json(minimal_manifest_json().expect("build manifest json")),
        )]))
        .expect("start mock registry");

        // ACT
        let annotations =
            pull::annotations(&registry.reference("repo", "test"), &Arch::Amd64, None)
                .expect("read annotations");

        // ASSERT
        assert!(annotations.is_empty());
    }

    #[test]
    fn stream_files_rejects_non_utf8_manifest_response() {
        // ARRANGE
        let registry = MockRegistry::start(HashMap::from([get(
            "/v2/repo/manifests/test",
            HttpResponse::json(vec![0xff, 0xfe, 0xfd]),
        )]))
        .expect("start mock registry");

        // ACT
        let error = pull::files(
            &registry.reference("repo", "test"),
            &Arch::Amd64,
            None,
            |_entry| Ok(()),
        )
        .expect_err("stream should fail");

        // ASSERT
        assert!(matches!(error, KociError::NetworkError(_)));
    }

    #[test]
    fn stream_files_rejects_invalid_manifest_json() {
        // ARRANGE
        let registry = MockRegistry::start(HashMap::from([get(
            "/v2/repo/manifests/test",
            HttpResponse::json(b"not json".to_vec()),
        )]))
        .expect("start mock registry");

        // ACT
        let error = pull::files(
            &registry.reference("repo", "test"),
            &Arch::Amd64,
            None,
            |_entry| Ok(()),
        )
        .expect_err("stream should fail");

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
        let registry = MockRegistry::start(HashMap::from([
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
            .arg("--annotation")
            .arg("dev.muak.sig")
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
        assert!(
            manifest
                .get("annotations")
                .and_then(|annotations| annotations.get("dev.muak.sig"))
                .and_then(Value::as_str)
                .is_some()
        );
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
            .arg("--annotation")
            .arg("dev.muak.sig")
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

    #[test]
    fn annotate_single_manifest_measures_pushed_blobs() {
        // ARRANGE
        let layer = layer_archive(&[("vmlinuz", b"kernel-bytes\n"), ("cmdline", b"args\n")])
            .expect("build layer archive");
        let layer_digest = sha256_digest(&layer);
        let manifest =
            annotated_manifest_json(&layer_digest, layer.len(), &[(SIG_ANNOTATION, "AA")])
                .expect("build annotated manifest json");
        let tag_path = "/v2/repo/manifests/test";
        let registry = MockRegistry::start(HashMap::from([
            get(tag_path, HttpResponse::json(manifest)),
            get(
                format!("/v2/repo/blobs/{layer_digest}"),
                HttpResponse::octet_stream(layer),
            ),
            put(tag_path, HttpResponse::ok()),
        ]))
        .expect("start mock registry");

        // ACT
        annotations::sizes(&registry.reference("repo", "test"), SIZES_ANNOTATION, &[])
            .expect("annotate image");

        // ASSERT
        let request = required_request(&registry, "PUT", tag_path);
        let annotated: Value =
            serde_json::from_slice(&request.body).expect("parse annotated manifest body");
        let annotations = annotated
            .get("annotations")
            .expect("annotated manifest must keep annotations");
        assert_eq!(
            annotations.get("dev.muak.sig").and_then(Value::as_str),
            Some("AA"),
            "unrelated annotations must be preserved"
        );
        assert_eq!(
            annotations.get("dev.muak.sizes").and_then(Value::as_str),
            Some(r#"{"cmdline":5,"vmlinuz":13}"#),
        );
        assert_eq!(
            annotated
                .get("layers")
                .and_then(Value::as_array)
                .and_then(|layers| layers.first())
                .and_then(|layer| layer.get("digest"))
                .and_then(Value::as_str),
            Some(layer_digest.as_str()),
            "layer digests must be untouched"
        );
    }

    #[test]
    fn annotate_excludes_entry_prefixes() {
        // ARRANGE
        let layer = layer_archive(&[
            ("vmlinuz", b"kernel-bytes\n"),
            ("lib/modules/x.ko", b"module\n"),
        ])
        .expect("build layer archive");
        let layer_digest = sha256_digest(&layer);
        let manifest = manifest_json(&layer_digest, layer.len()).expect("build manifest json");
        let tag_path = "/v2/repo/manifests/test";
        let registry = MockRegistry::start(HashMap::from([
            get(tag_path, HttpResponse::json(manifest)),
            get(
                format!("/v2/repo/blobs/{layer_digest}"),
                HttpResponse::octet_stream(layer),
            ),
            put(tag_path, HttpResponse::ok()),
        ]))
        .expect("start mock registry");

        // ACT
        annotations::sizes(
            &registry.reference("repo", "test"),
            SIZES_ANNOTATION,
            &["lib/modules".to_owned()],
        )
        .expect("annotate image");

        // ASSERT
        let request = required_request(&registry, "PUT", tag_path);
        let annotated: Value =
            serde_json::from_slice(&request.body).expect("parse annotated manifest body");
        assert_eq!(
            annotated
                .get("annotations")
                .and_then(|annotations| annotations.get("dev.muak.sizes"))
                .and_then(Value::as_str),
            Some(r#"{"vmlinuz":13}"#),
        );
    }

    #[test]
    fn annotate_annotates_every_platform_manifest_of_an_index() {
        // ARRANGE
        let first_layer = layer_archive(&[("vmlinuz", b"first\n")]).expect("build first layer");
        let second_layer = layer_archive(&[("vmlinuz", b"second\n")]).expect("build second layer");
        let first_digest = sha256_digest(&first_layer);
        let second_digest = sha256_digest(&second_layer);
        let first_platform =
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let second_platform =
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let index = index_for_arches_json(&[
            (first_platform, "amd64", "linux"),
            (second_platform, "arm64", "linux"),
        ])
        .expect("build index json");
        let tag_path = "/v2/repo/manifests/test";
        let registry = MockRegistry::start(HashMap::from([
            get(tag_path, HttpResponse::index(index)),
            get(
                format!("/v2/repo/manifests/{first_platform}"),
                HttpResponse::json(manifest_json(&first_digest, first_layer.len()).expect("m")),
            ),
            get(
                format!("/v2/repo/manifests/{second_platform}"),
                HttpResponse::json(manifest_json(&second_digest, second_layer.len()).expect("m")),
            ),
            get(
                format!("/v2/repo/blobs/{first_digest}"),
                HttpResponse::octet_stream(first_layer),
            ),
            get(
                format!("/v2/repo/blobs/{second_digest}"),
                HttpResponse::octet_stream(second_layer),
            ),
            put(
                format!("/v2/repo/manifests/{first_platform}"),
                HttpResponse::ok(),
            ),
            put(
                format!("/v2/repo/manifests/{second_platform}"),
                HttpResponse::ok(),
            ),
        ]))
        .expect("start mock registry");

        // ACT
        annotations::sizes(&registry.reference("repo", "test"), SIZES_ANNOTATION, &[])
            .expect("annotate image");

        // ASSERT
        for (digest, size) in [(first_platform, 6), (second_platform, 7)] {
            let request =
                required_request(&registry, "PUT", &format!("/v2/repo/manifests/{digest}"));
            let annotated: Value =
                serde_json::from_slice(&request.body).expect("parse annotated manifest body");
            assert_eq!(
                annotated
                    .get("annotations")
                    .and_then(|annotations| annotations.get("dev.muak.sizes"))
                    .and_then(Value::as_str),
                Some(format!(r#"{{"vmlinuz":{size}}}"#).as_str()),
            );
        }
        assert!(
            registry
                .request("PUT", tag_path)
                .expect("read log")
                .is_none(),
            "the index itself must not be rewritten"
        );
    }

    #[test]
    fn cli_annotate_writes_sizes_annotation() {
        // ARRANGE
        let layer = layer_archive(&[("stub.efi", b"pe-binary\n")]).expect("build layer archive");
        let layer_digest = sha256_digest(&layer);
        let manifest = manifest_json(&layer_digest, layer.len()).expect("build manifest json");
        let tag_path = "/v2/repo/manifests/test";
        let registry = MockRegistry::start(HashMap::from([
            get(tag_path, HttpResponse::json(manifest)),
            get(
                format!("/v2/repo/blobs/{layer_digest}"),
                HttpResponse::octet_stream(layer),
            ),
            put(tag_path, HttpResponse::ok()),
        ]))
        .expect("start mock registry");

        // ACT
        let output = Command::new(koci_bin())
            .arg("annotate")
            .arg("--image")
            .arg(registry.reference("repo", "test"))
            .arg("--annotation")
            .arg("dev.muak.sizes")
            .output()
            .expect("run koci annotate");

        // ASSERT
        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let request = required_request(&registry, "PUT", tag_path);
        let annotated: Value =
            serde_json::from_slice(&request.body).expect("parse annotated manifest body");
        assert_eq!(
            annotated
                .get("annotations")
                .and_then(|annotations| annotations.get("dev.muak.sizes"))
                .and_then(Value::as_str),
            Some(r#"{"stub.efi":10}"#),
        );
    }
}
