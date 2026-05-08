//! OCI registry authentication token fetching.

use serde::Deserialize;

use crate::error::{ImagerError, Result};
use crate::oci::http::{HttpClient, collect_body, get};

/// JSON response returned by OCI token endpoints.
#[derive(Deserialize)]
pub(crate) struct TokenResponse {
    pub token: String,
}

/// Return the token endpoint URL for known public registries, or `None` for private ones.
pub(crate) fn get_token_url(registry: &str, name: &str) -> Option<String> {
    if registry == "ghcr.io" {
        Some(format!(
            "https://ghcr.io/token?scope=repository:{}:pull",
            name
        ))
    } else if registry.contains("docker.io") {
        Some(format!(
            "https://auth.docker.io/token?service=registry.docker.io&scope=repository:{}:pull",
            name
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
    let Some(token_url) = get_token_url(registry, name) else {
        return Ok(None);
    };

    fetch_auth_token_from_url(client, &token_url).await
}

async fn fetch_auth_token_from_url(client: &HttpClient, token_url: &str) -> Result<Option<String>> {
    let resp = match get(client, token_url, None, &[]).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Warning: Failed to get auth token: {}", e);
            return Ok(None);
        }
    };

    let body = collect_body(resp).await?;
    Ok(Some(parse_token_response(&body)?))
}

fn parse_token_response(body: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(body).map_err(|e| {
        ImagerError::NetworkError(format!("Auth token response is not UTF-8: {}", e))
    })?;
    let token_resp: TokenResponse = serde_json::from_str(text)?;
    Ok(token_resp.token)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;
    use crate::oci::http::build_client;

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
            let status = status.to_string();
            let body = body.to_vec();

            let handle = thread::spawn(move || {
                let (mut stream, _) = listener.accept().expect("accept test client");
                let mut request = [0_u8; 1024];
                let _ = stream.read(&mut request).expect("read test request");

                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write test response headers");
                stream.write_all(&body).expect("write test response body");
            });

            Self {
                address,
                handle: Some(handle),
            }
        }

        fn url(&self) -> String {
            format!("http://{}/token", self.address)
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            if let Some(handle) = self.handle.take() {
                handle.join().expect("join test server thread");
            }
        }
    }

    #[test]
    fn get_token_url_matches_supported_registries() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(
            get_token_url("ghcr.io", "org/image"),
            Some("https://ghcr.io/token?scope=repository:org/image:pull".to_string())
        );
        assert_eq!(
            get_token_url("registry-1.docker.io", "library/alpine"),
            Some(
                "https://auth.docker.io/token?service=registry.docker.io&scope=repository:library/alpine:pull"
                    .to_string()
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
            Err(ImagerError::NetworkError(_))
        ));
    }

    #[test]
    fn parse_token_response_rejects_invalid_json() {
        // ARRANGE
        let body = br#"{"access_token":"abc123"}"#;

        // ACT / ASSERT
        assert!(matches!(
            parse_token_response(body),
            Err(ImagerError::SerializationError(_))
        ));
    }

    #[tokio::test]
    async fn fetch_auth_token_skips_private_registries() {
        // ARRANGE
        let client = build_client().expect("build HTTP client");

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
        let client = build_client().expect("build HTTP client");

        // ACT / ASSERT
        assert_eq!(
            fetch_auth_token_from_url(&client, &server.url())
                .await
                .expect("fetch auth token"),
            Some("abc123".to_string())
        );
    }

    #[tokio::test]
    async fn fetch_auth_token_from_url_returns_none_on_request_failure() {
        // ARRANGE
        let client = build_client().expect("build HTTP client");

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
        let client = build_client().expect("build HTTP client");

        // ACT / ASSERT
        assert!(matches!(
            fetch_auth_token_from_url(&client, &server.url()).await,
            Err(ImagerError::SerializationError(_))
        ));
    }
}
