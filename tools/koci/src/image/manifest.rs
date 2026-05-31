//! OCI manifest fetching, parsing, and platform selection.

use crate::error::{KociError, Result};
use crate::image::{ImageReference, OciDescriptor, OciManifest};
use crate::registry::OCI_MANIFEST_ACCEPT_HEADERS;
use crate::registry::http::{HttpClient, collect_body, get};

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
    token: Option<&str>,
) -> Result<String> {
    let resp = get(client, manifest_url, token, OCI_MANIFEST_ACCEPT_HEADERS).await?;
    let body = collect_body(resp).await?;
    String::from_utf8(body.to_vec()).map_err(|error| {
        KociError::NetworkError(format!("Manifest response is not UTF-8: {error}"))
    })
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

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::thread;

    use super::*;
    use crate::image::{ImageReference, Platform};
    use crate::registry::http::build_client;

    struct TestServer {
        address: String,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl TestServer {
        fn spawn(status: &str, body: &[u8]) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
            let address = listener
                .local_addr()
                .expect("get test server address")
                .to_string();
            let status = status.to_owned();
            let body = body.to_vec();

            let handle = thread::spawn(move || serve_manifest(&listener, &status, &body));

            Self {
                address,
                handle: Some(handle),
            }
        }

        fn url(&self) -> String {
            format!("http://{}/manifest", self.address)
        }
    }

    fn serve_manifest(listener: &TcpListener, status: &str, body: &[u8]) {
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
        stream.write_all(body).expect("write test response body");
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
                Some(crate::host_oci_arch()),
                Some("windows"),
            ),
            descriptor("sha256:match", Some(crate::host_oci_arch()), Some("linux")),
            descriptor("sha256:wrong-arch", Some("arm64"), Some("linux")),
        ];

        // ACT
        let selected =
            select_platform(&manifests, crate::host_oci_arch()).expect("select matching manifest");

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
}
