//! Registry manifest fetching and pushing.

use hyper::body::Bytes;
use oci::media::OCI_MANIFEST_ACCEPT_HEADERS;
use oci::reference::Image;

use crate::error::{KociError, Result};
use crate::registry::http::{self, HttpClient, collect_body, get};
use crate::registry::session::Session;

/// Build the manifest URL for a given image reference and tag or digest.
pub(crate) fn build_url(image: &Image, reference: &str) -> String {
    format!(
        "{}://{}/v2/{}/manifests/{}",
        image.scheme(),
        image.registry,
        image.name,
        reference
    )
}

/// Fetch and return the raw manifest JSON from the registry.
///
/// # Errors
///
/// Returns an error when the request fails or the body is not UTF-8.
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

/// Push a manifest to the registry via PUT.
///
/// # Errors
///
/// Returns an error when the request fails or the registry rejects the manifest.
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

    #[test]
    fn build_url_uses_registry_scheme_and_reference() {
        // ARRANGE
        let image = Image {
            registry: "127.0.0.1:5000".to_owned(),
            name: "repo/name".to_owned(),
            manifest_ref: "test".to_owned(),
        };

        // ACT / ASSERT
        assert_eq!(
            build_url(&image, "sha256:abc"),
            "http://127.0.0.1:5000/v2/repo/name/manifests/sha256:abc"
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
