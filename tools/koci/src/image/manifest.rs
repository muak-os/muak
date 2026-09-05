//! OCI manifest fetching, parsing, platform selection, and writing.

use hyper::body::Bytes;

use crate::error::{KociError, Result};
use crate::image::{ImageReference, OciDescriptor, OciManifest};
use crate::registry::OCI_MANIFEST_ACCEPT_HEADERS;
use crate::registry::http::{self, HttpClient, collect_body, get};
use crate::registry::session::Session;

/// Build the manifest URL for a given image reference and tag or digest.
pub(crate) fn build_url(image_ref: &ImageReference, reference: &str) -> String {
    format!(
        "{}://{}/v2/{}/manifests/{}",
        image_ref.scheme(),
        image_ref.registry,
        image_ref.name,
        reference
    )
}

/// Fetch and return the raw manifest JSON from the registry.
pub(crate) async fn fetch(
    client: &HttpClient,
    manifest_url: &str,
    authorization: Option<&str>,
) -> Result<String> {
    let resp = get(
        client,
        manifest_url,
        authorization,
        OCI_MANIFEST_ACCEPT_HEADERS,
    )
    .await?;
    let body = collect_body(resp).await?;
    match core::str::from_utf8(&body) {
        Ok(text) => Ok(text.to_owned()),
        Err(error) => Err(KociError::NetworkError(format!(
            "Manifest response is not UTF-8: {error}"
        ))),
    }
}

/// Parse manifest JSON into an [`OciManifest`].
pub(crate) fn parse(json: &str) -> Result<OciManifest> {
    serde_json::from_str(json)
        .map_err(|error| KociError::OciParseError(format!("Failed to parse manifest: {error}")))
}

/// Select the matching platform manifest for the requested target architecture.
pub(crate) fn select_platform<'a>(
    manifests: &'a [OciDescriptor],
    target_arch: &str,
) -> Result<&'a OciDescriptor> {
    manifests
        .iter()
        .find(|descriptor| {
            descriptor.platform.as_ref().is_some_and(|platform| {
                platform.architecture.as_deref() == Some(target_arch)
                    && platform.os.as_deref() == Some("linux")
            })
        })
        .ok_or_else(|| {
            KociError::InvalidOciFormat(format!(
                "No linux/{target_arch} manifest found in manifest list"
            ))
        })
}

/// Set one manifest annotation, preserving the others, and serialize the manifest with its content type.
pub(crate) fn with_annotation(
    manifest_json: &str,
    key: &str,
    value: &str,
) -> Result<(Bytes, String)> {
    let mut manifest_value: serde_json::Value =
        serde_json::from_str(manifest_json).map_err(|error| {
            KociError::OciParseError(format!("Failed to parse manifest JSON: {error}"))
        })?;

    manifest_value
        .as_object_mut()
        .ok_or_else(|| KociError::InvalidOciFormat("Manifest is not a JSON object".to_owned()))?
        .entry("annotations")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .ok_or_else(|| {
            KociError::InvalidOciFormat("Manifest annotations is not a JSON object".to_owned())
        })?
        .insert(key.to_owned(), serde_json::Value::String(value.to_owned()));

    let content_type = manifest_value
        .get("mediaType")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("application/vnd.oci.image.manifest.v1+json")
        .to_owned();

    let body = serde_json::to_vec(&manifest_value)?;

    Ok((Bytes::from(body), content_type))
}

/// Push a manifest to the registry via PUT.
pub(crate) async fn put(
    session: &Session,
    manifest_ref: &str,
    content_type: &str,
    body: Bytes,
) -> Result<()> {
    let url = build_url(&session.image, manifest_ref);
    http::put(
        &session.client,
        &url,
        session.authorization(),
        content_type,
        body,
    )
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::thread;

    use super::*;
    use crate::arch;
    use crate::image::{ImageReference, Platform};
    use crate::registry::auth::Access;
    use crate::registry::http::build_client;

    struct TestServer {
        address: String,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl TestServer {
        fn spawn(status: &str, body: &[u8]) -> Self {
            Self::spawn_responses(&[(status, body)])
        }

        fn spawn_responses(responses: &[(&str, &[u8])]) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
            let address = listener
                .local_addr()
                .expect("get test server address")
                .to_string();
            let responses = responses
                .iter()
                .map(owned_response)
                .collect::<Vec<(String, Vec<u8>)>>();
            let handle = thread::spawn(move || serve_responses(&listener, responses));

            Self {
                address,
                handle: Some(handle),
            }
        }

        fn url(&self) -> String {
            format!("http://{}/manifest", self.address)
        }
    }

    fn owned_response(response: &(&str, &[u8])) -> (String, Vec<u8>) {
        let (status, body) = *response;

        (status.to_owned(), body.to_vec())
    }

    fn serve_responses(listener: &TcpListener, responses: Vec<(String, Vec<u8>)>) {
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().expect("accept test client");
            let mut request = [0_u8; 1024];
            let _: usize = stream.read(&mut request).expect("read test request");

            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/vnd.oci.image.manifest.v1+json\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("write test response headers");
            stream.write_all(&body).expect("write test response body");
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            join_test_server(self.handle.take());
        }
    }

    fn join_test_server(handle: Option<thread::JoinHandle<()>>) {
        let Some(handle) = handle else {
            return;
        };
        handle.join().expect("join test server thread");
    }

    fn descriptor(digest: &str, architecture: Option<&str>, os: Option<&str>) -> OciDescriptor {
        OciDescriptor {
            media_type: None,
            digest: digest.to_owned(),
            platform: Some(Platform {
                architecture: architecture.map(str::to_owned),
                os: os.map(str::to_owned),
            }),
        }
    }

    #[test]
    fn build_url_uses_registry_scheme_and_reference() {
        // ARRANGE
        let image_ref = ImageReference {
            registry: "127.0.0.1:5000".to_owned(),
            name: "repo/name".to_owned(),
            manifest_ref: "test".to_owned(),
        };

        // ACT / ASSERT
        assert_eq!(
            build_url(&image_ref, "sha256:abc"),
            "http://127.0.0.1:5000/v2/repo/name/manifests/sha256:abc"
        );
    }

    #[test]
    fn parse_invalid_manifest_returns_error() {
        // ARRANGE / ACT
        let result = parse("not json");

        // ASSERT
        assert!(matches!(result, Err(KociError::OciParseError(_))));
    }

    #[test]
    fn parse_manifest_with_layers_and_platforms() {
        // ARRANGE
        let manifest_json = r#"{
            "schemaVersion": 2,
            "layers": [{
                "mediaType": "application/vnd.oci.image.layer.v1.tar",
                "digest": "sha256:abc"
            }],
            "manifests": [{
                "digest": "sha256:def",
                "platform": {
                    "architecture": "amd64",
                    "os": "linux"
                }
            }]
        }"#;

        // ACT
        let manifest = parse(manifest_json).expect("parse manifest");

        // ASSERT
        assert_eq!(manifest.layers.len(), 1);
        assert_eq!(
            manifest
                .layers
                .first()
                .and_then(|layer| layer.media_type.as_deref()),
            Some("application/vnd.oci.image.layer.v1.tar")
        );
        assert_eq!(manifest.manifests.len(), 1);
        assert_eq!(
            manifest
                .manifests
                .first()
                .expect("manifest should include a platform descriptor")
                .platform
                .as_ref()
                .and_then(|platform| platform.architecture.as_deref()),
            Some("amd64")
        );
    }

    #[tokio::test]
    async fn fetch_manifest_returns_body_text() {
        // ARRANGE
        let server = TestServer::spawn("200 OK", br#"{"schemaVersion":2}"#);
        let client = build_client();

        // ACT
        let manifest = fetch(&client, &server.url(), Some("token"))
            .await
            .expect("fetch manifest");

        // ASSERT
        assert_eq!(manifest, "{\"schemaVersion\":2}");
    }

    #[tokio::test]
    async fn fetch_manifest_rejects_non_utf8_body() {
        // ARRANGE
        let server = TestServer::spawn("200 OK", &[0xff, 0xfe, 0xfd]);
        let client = build_client();

        // ACT
        let error = fetch(&client, &server.url(), None)
            .await
            .expect_err("fetch should fail");

        // ASSERT
        assert!(matches!(error, KociError::NetworkError(_)));
    }

    #[tokio::test]
    async fn fetch_manifest_propagates_http_failures() {
        // ARRANGE
        let server = TestServer::spawn("404 Not Found", b"missing");
        let client = build_client();

        // ACT
        let error = fetch(&client, &server.url(), None)
            .await
            .expect_err("fetch should fail");

        // ASSERT
        assert!(matches!(error, KociError::DownloadError(_)));
    }

    #[test]
    fn select_platform_ignores_descriptor_without_platform() {
        // ARRANGE
        let manifests = vec![
            OciDescriptor {
                media_type: None,
                digest: "sha256:no-platform".to_owned(),
                platform: None,
            },
            descriptor("sha256:match", Some("amd64"), Some("linux")),
        ];

        // ACT
        let selected = select_platform(&manifests, "amd64").expect("select matching manifest");

        // ASSERT
        assert_eq!(selected.digest, "sha256:match");
    }

    #[test]
    fn select_platform_rejects_non_linux_match() {
        // ARRANGE
        let manifests = vec![descriptor("sha256:wrong-os", Some("amd64"), Some("darwin"))];

        // ACT
        let error = select_platform(&manifests, "amd64").expect_err("selection should fail");

        // ASSERT
        assert!(matches!(error, KociError::InvalidOciFormat(_)));
    }

    #[test]
    fn select_platform_prefers_host_linux_match() {
        // ARRANGE
        let manifests = vec![
            descriptor(
                "sha256:wrong-os",
                Some(arch::host().as_str()),
                Some("windows"),
            ),
            descriptor("sha256:match", Some(arch::host().as_str()), Some("linux")),
            descriptor("sha256:wrong-arch", Some("arm64"), Some("linux")),
        ];

        // ACT
        let selected =
            select_platform(&manifests, arch::host().as_str()).expect("select matching manifest");

        // ASSERT
        assert_eq!(selected.digest, "sha256:match");
    }

    #[test]
    fn select_platform_errors_without_matching_target() {
        // ARRANGE
        let manifests = vec![
            descriptor("sha256:first", Some("arm64"), Some("windows")),
            descriptor("sha256:second", Some("386"), Some("linux")),
        ];

        // ACT
        let result = select_platform(&manifests, "amd64");

        // ASSERT
        assert!(matches!(result, Err(KociError::InvalidOciFormat(_))));
    }

    #[test]
    fn select_platform_errors_for_empty_manifest_list() {
        // ARRANGE / ACT
        let result = select_platform(&[], "amd64");

        // ASSERT
        assert!(matches!(result, Err(KociError::InvalidOciFormat(_))));
    }

    #[test]
    fn with_annotation_sets_key_and_preserves_others() {
        // ARRANGE
        let manifest_json = r#"{"schemaVersion":2,"annotations":{"dev.muak.sig":"AA"},"mediaType":"application/vnd.oci.image.manifest.v1+json","layers":[]}"#;

        // ACT
        let (body, content_type) =
            with_annotation(manifest_json, "dev.muak.sizes", "{}").expect("annotate manifest");

        // ASSERT
        let annotated: serde_json::Value =
            serde_json::from_slice(&body).expect("parse annotated manifest");
        let annotations = annotated
            .get("annotations")
            .and_then(serde_json::Value::as_object)
            .expect("annotated manifest must keep its annotations object");
        assert_eq!(
            annotations
                .get("dev.muak.sig")
                .and_then(serde_json::Value::as_str),
            Some("AA")
        );
        assert_eq!(
            annotations
                .get("dev.muak.sizes")
                .and_then(serde_json::Value::as_str),
            Some("{}")
        );
        assert_eq!(content_type, "application/vnd.oci.image.manifest.v1+json");
    }

    #[test]
    fn with_annotation_creates_annotations_map_and_defaults_content_type() {
        // ARRANGE
        let manifest_json = r#"{"schemaVersion":2,"layers":[]}"#;

        // ACT
        let (body, content_type) =
            with_annotation(manifest_json, "dev.muak.sizes", "{}\"").expect("annotate manifest");

        // ASSERT
        let annotated: serde_json::Value =
            serde_json::from_slice(&body).expect("parse annotated manifest");
        assert_eq!(
            annotated
                .get("annotations")
                .and_then(|annotations| annotations.get("dev.muak.sizes"))
                .and_then(serde_json::Value::as_str),
            Some("{}\"")
        );
        assert_eq!(content_type, "application/vnd.oci.image.manifest.v1+json");
    }

    #[test]
    fn with_annotation_rejects_non_object_manifest() {
        // ARRANGE / ACT
        let error =
            with_annotation("[]", "dev.muak.sizes", "{}").expect_err("annotate should fail");

        // ASSERT
        assert!(matches!(error, KociError::InvalidOciFormat(_)));
    }

    #[test]
    fn with_annotation_rejects_non_object_annotations() {
        // ARRANGE
        let manifest_json = r#"{"schemaVersion":2,"annotations":[],"layers":[]}"#;

        // ACT
        let error = with_annotation(manifest_json, "dev.muak.sizes", "{}")
            .expect_err("annotate should fail");

        // ASSERT
        assert!(matches!(error, KociError::InvalidOciFormat(_)));
    }

    #[tokio::test]
    async fn put_manifest_propagates_failures() {
        // ARRANGE
        let server = TestServer::spawn_responses(&[
            ("200 OK", b""),                     // auth ping
            ("405 Method Not Allowed", b"nope"), // manifest PUT
        ]);
        let reference = format!("{}/repo:test", server.address);
        let session = Session::new(&reference, Access::Pull, None)
            .await
            .expect("build session");

        // ACT
        let error = put(
            &session,
            "test",
            "application/vnd.oci.image.manifest.v1+json",
            Bytes::from_static(b"{}"),
        )
        .await
        .expect_err("put manifest should fail");

        // ASSERT
        assert!(matches!(error, KociError::DownloadError(_)));
    }
}
