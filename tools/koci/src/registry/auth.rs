//! OCI registry authentication token fetching.

use serde::Deserialize;

use crate::error::{KociError, Result};
use crate::registry::http::{HttpClient, collect_body, get};

/// JSON response returned by OCI token endpoints.
#[derive(Deserialize)]
pub(crate) struct TokenResponse {
    pub token: String,
}

/// Return the token endpoint URL for known public registries, or `None` for private ones.
pub(crate) fn get_token_url(registry: &str, name: &str) -> Option<String> {
    if registry == "ghcr.io" {
        Some(format!(
            "https://ghcr.io/token?scope=repository:{name}:pull"
        ))
    } else if registry.contains("docker.io") {
        Some(format!(
            "https://auth.docker.io/token?service=registry.docker.io&scope=repository:{name}:pull"
        ))
    } else {
        None
    }
}

/// Fetch a Bearer token for the given registry and image name.
pub(crate) async fn fetch_auth_token(
    client: &HttpClient,
    registry: &str,
    name: &str,
) -> Result<Option<String>> {
    match get_token_url(registry, name) {
        Some(token_url) => fetch_auth_token_from_url(client, &token_url).await,
        None => Ok(None),
    }
}

async fn fetch_auth_token_from_url(client: &HttpClient, token_url: &str) -> Result<Option<String>> {
    let resp = match get(client, token_url, None, &[]).await {
        Ok(response) => response,
        Err(error) => {
            eprintln!("Warning: Failed to get auth token: {error}");
            return Ok(None);
        }
    };

    let body = collect_body(resp).await?;
    Ok(Some(parse_token_response(&body)?))
}

fn parse_token_response(body: &[u8]) -> Result<String> {
    let text = core::str::from_utf8(body).map_err(|error| {
        KociError::NetworkError(format!("Auth token response is not UTF-8: {error}"))
    })?;
    let token_resp: TokenResponse = serde_json::from_str(text)?;
    Ok(token_resp.token)
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::thread;

    use super::*;
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

            let handle = thread::spawn(move || serve_auth(&listener, &status, &body));

            Self {
                address,
                handle: Some(handle),
            }
        }

        fn url(&self) -> String {
            format!("http://{}/token", self.address)
        }
    }

    fn serve_auth(listener: &TcpListener, status: &str, body: &[u8]) {
        let (mut stream, _) = listener.accept().expect("accept test client");
        let mut request = [0_u8; 1024];
        let _: usize = stream.read(&mut request).expect("read test request");

        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
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

    #[test]
    fn get_token_url_matches_supported_registries() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(
            get_token_url("ghcr.io", "org/image"),
            Some("https://ghcr.io/token?scope=repository:org/image:pull".to_owned())
        );
        assert_eq!(
            get_token_url("registry-1.docker.io", "library/alpine"),
            Some(
                "https://auth.docker.io/token?service=registry.docker.io&scope=repository:library/alpine:pull"
                    .to_owned()
            )
        );
        assert_eq!(get_token_url("127.0.0.1:5000", "repo"), None);
    }

    #[test]
    fn parse_token_response_accepts_valid_json() {
        // ARRANGE
        let body = br#"{"token":"abc123"}"#;

        // ACT / ASSERT
        assert_eq!(
            parse_token_response(body).expect("parse token response"),
            "abc123"
        );
    }

    #[test]
    fn parse_token_response_rejects_non_utf8_body() {
        // ARRANGE
        let body = [0xff, 0xfe, 0xfd];

        // ACT / ASSERT
        assert!(matches!(
            parse_token_response(&body),
            Err(KociError::NetworkError(_))
        ));
    }

    #[test]
    fn parse_token_response_rejects_invalid_json() {
        // ARRANGE
        let body = br#"{"access_token":"abc123"}"#;

        // ACT / ASSERT
        assert!(matches!(
            parse_token_response(body),
            Err(KociError::SerializationError(_))
        ));
    }

    #[tokio::test]
    async fn fetch_auth_token_skips_private_registries() {
        // ARRANGE
        let client = build_client();

        // ACT / ASSERT
        assert!(matches!(
            fetch_auth_token(&client, "127.0.0.1:5000", "repo").await,
            Ok(None)
        ));
    }

    #[tokio::test]
    async fn fetch_auth_token_from_url_parses_token_body() {
        // ARRANGE
        let server = TestServer::spawn("200 OK", br#"{"token":"abc123"}"#);
        let client = build_client();

        // ACT / ASSERT
        assert_eq!(
            fetch_auth_token_from_url(&client, &server.url())
                .await
                .expect("fetch auth token"),
            Some("abc123".to_owned())
        );
    }

    #[tokio::test]
    async fn fetch_auth_token_from_url_returns_none_on_request_failure() {
        // ARRANGE
        let client = build_client();

        // ACT / ASSERT
        assert!(matches!(
            fetch_auth_token_from_url(&client, "http://127.0.0.1:9/token").await,
            Ok(None)
        ));
    }

    #[tokio::test]
    async fn fetch_auth_token_from_url_propagates_invalid_json() {
        // ARRANGE
        let server = TestServer::spawn("200 OK", br#"{"access_token":"abc123"}"#);
        let client = build_client();

        // ACT / ASSERT
        assert!(matches!(
            fetch_auth_token_from_url(&client, &server.url()).await,
            Err(KociError::SerializationError(_))
        ));
    }
}
