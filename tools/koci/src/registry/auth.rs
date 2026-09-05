//! OCI registry authentication per the distribution token-auth flow.

use base64ct::{Base64, Encoding as _};
use hyper::Response;
use hyper::body::Incoming;
use hyper::http::StatusCode;
use serde::Deserialize;

use crate::error::{KociError, Result};
use crate::registry::challenge::Challenge;
use crate::registry::http::{self, HttpClient};

/// Environment variable carrying the registry username.
const USERNAME_ENV: &str = "KOCI_REGISTRY_USERNAME";
/// Environment variable carrying the registry password or token.
const PASSWORD_ENV: &str = "KOCI_REGISTRY_PASSWORD";
/// `WWW-Authenticate` response header name.
const WWW_AUTHENTICATE: &str = "WWW-Authenticate";

/// Registry access level to request from the token endpoint.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Access {
    /// Read-only access to one repository.
    Pull,
    /// Read and write access to one repository.
    PullPush,
}

impl Access {
    /// Scope actions covered by this access level.
    fn actions(self) -> &'static str {
        match self {
            Self::Pull => "pull",
            Self::PullPush => "pull,push",
        }
    }
}

/// Registry credentials for the token endpoint's Basic authentication.
#[derive(Clone, Debug)]
pub(crate) struct Credentials {
    username: String,
    password: String,
}

impl Credentials {
    /// Create credentials from a username and a password or token.
    pub(crate) fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: password.into(),
        }
    }

    /// Read credentials from [`USERNAME_ENV`] and [`PASSWORD_ENV`].
    pub(crate) fn from_env() -> Option<Self> {
        Self::from_parts(
            std::env::var(USERNAME_ENV).ok().as_deref(),
            std::env::var(PASSWORD_ENV).ok().as_deref(),
        )
    }

    /// Build credentials from optional raw values, ignoring empty ones.
    fn from_parts(username: Option<&str>, password: Option<&str>) -> Option<Self> {
        Some(Self::new(
            username.filter(|value| !value.is_empty())?,
            password.filter(|value| !value.is_empty())?,
        ))
    }

    /// HTTP Basic authorization header value for these credentials.
    fn basic_header(&self) -> String {
        let encoded =
            Base64::encode_string(format!("{}:{}", self.username, self.password).as_bytes());
        format!("Basic {encoded}")
    }
}

/// Resolve the `Authorization` header value for requests against one image.
///
/// # Errors
///
/// Returns [`KociError::AuthError`] when the registry challenges requests and
/// the challenge cannot be answered.
pub(crate) async fn authenticate(
    client: &HttpClient,
    scheme: &str,
    registry: &str,
    name: &str,
    access: Access,
    credentials: Option<&Credentials>,
) -> Result<Option<String>> {
    let ping_url = format!("{scheme}://{registry}/v2/");
    let response = http::get_any_status(client, &ping_url, None, &[]).await?;

    if response.status().is_success() {
        return Ok(None);
    }
    if response.status() != StatusCode::UNAUTHORIZED {
        eprintln!(
            "Registry auth probe returned HTTP {}; continuing unauthenticated",
            response.status()
        );
        return Ok(None);
    }

    let challenge = pick_challenge(&response, registry)?;
    match challenge.scheme.as_str() {
        "bearer" => bearer_token(client, &challenge, name, access, credentials, registry)
            .await
            .map(Some),
        "basic" => {
            let credentials = credentials.ok_or_else(|| {
                auth_error(
                    registry,
                    "registry requires Basic authentication but none are configured",
                )
            })?;
            Ok(Some(credentials.basic_header()))
        }
        other => Err(auth_error(
            registry,
            format!("unsupported authentication scheme '{other}'"),
        )),
    }
}

/// Pick the best parsable challenge from a `401` response, preferring Bearer.
fn pick_challenge(response: &Response<Incoming>, registry: &str) -> Result<Challenge> {
    let challenges: Vec<Challenge> = response
        .headers()
        .get_all(WWW_AUTHENTICATE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(Challenge::parse)
        .collect();

    challenges
        .iter()
        .find(|challenge| challenge.scheme == "bearer")
        .or_else(|| challenges.first())
        .cloned()
        .ok_or_else(|| {
            auth_error(
                registry,
                "401 response carries no parsable WWW-Authenticate challenge",
            )
        })
}

/// Fetch a scoped bearer token from the challenge's token realm.
async fn bearer_token(
    client: &HttpClient,
    challenge: &Challenge,
    name: &str,
    access: Access,
    credentials: Option<&Credentials>,
    registry: &str,
) -> Result<String> {
    let url = bearer_token_url(challenge, name, access, registry)?;
    let basic = credentials.map(Credentials::basic_header);
    let response = http::get(client, &url, basic.as_deref(), &[])
        .await
        .map_err(|error| auth_error(registry, format!("failed to fetch bearer token: {error}")))?;
    let body = http::collect_body(response).await?;

    parse_token_response(&body, registry)
}

/// Assemble the token endpoint URL with the repository scope and service.
fn bearer_token_url(
    challenge: &Challenge,
    name: &str,
    access: Access,
    registry: &str,
) -> Result<String> {
    let realm = challenge
        .param("realm")
        .ok_or_else(|| auth_error(registry, "bearer challenge is missing its realm"))?;
    let scope = percent_encode(&format!("repository:{name}:{}", access.actions()));

    let mut url = format!("{realm}?scope={scope}");
    if let Some(service) = challenge.param("service") {
        url.push_str("&service=");
        url.push_str(&percent_encode(service));
    }

    Ok(url)
}

/// Token endpoint response per the distribution spec.
#[derive(Deserialize)]
struct TokenResponse {
    token: Option<String>,
    access_token: Option<String>,
}

/// Extract the `Authorization` header value from a token endpoint response.
fn parse_token_response(body: &[u8], registry: &str) -> Result<String> {
    let text = core::str::from_utf8(body).map_err(|error| {
        auth_error(
            registry,
            format!("token endpoint response is not UTF-8: {error}"),
        )
    })?;
    let parsed: TokenResponse = serde_json::from_str(text).map_err(|error| {
        auth_error(
            registry,
            format!("invalid token endpoint response: {error}"),
        )
    })?;

    parsed
        .token
        .or(parsed.access_token)
        .map(|token| format!("Bearer {token}"))
        .ok_or_else(|| auth_error(registry, "token endpoint response carries no token"))
}

/// Build an [`KociError::AuthError`] for a registry.
fn auth_error(registry: &str, details: impl core::fmt::Display) -> KociError {
    KociError::AuthError {
        registry: registry.to_owned(),
        details: details.to_string(),
    }
}

/// Percent-encode a query parameter value, leaving RFC 3986 unreserved bytes.
fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        encode_byte(byte, &mut encoded);
    }

    encoded
}

/// Append one byte to `encoded`, percent-encoded when not RFC 3986 unreserved.
fn encode_byte(byte: u8, encoded: &mut String) {
    const HEX_DIGITS: &[char; 16] = &[
        '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'A', 'B', 'C', 'D', 'E', 'F',
    ];

    if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
        encoded.push(char::from(byte));
        return;
    }

    encoded.push('%');
    if let Some(digit) = HEX_DIGITS.get(usize::from(byte >> 4)) {
        encoded.push(*digit);
    }
    if let Some(digit) = HEX_DIGITS.get(usize::from(byte & 0x0F)) {
        encoded.push(*digit);
    }
}

#[cfg(test)]
mod tests {
    use core::fmt::Write as _;
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::thread;

    use super::*;
    use crate::image::ImageReference;
    use crate::registry::http::build_client;

    const BASIC_CHALLENGE: &str = "WWW-Authenticate: Basic realm=\"registry\"";

    /// Bearer challenge template; `{addr}` is filled in by [`TestServer::spawn`].
    const BEARER_CHALLENGE_TEMPLATE: &str =
        r#"WWW-Authenticate: Bearer realm="http://{addr}/token",service="repo""#;

    /// Serve canned raw HTTP responses, one per incoming connection.
    struct TestServer {
        reference: String,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl TestServer {
        /// Start the server; `{addr}` inside any response becomes its address.
        fn spawn(responses: &[&str]) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
            let address = listener.local_addr().expect("read test server address");
            let responses = responses
                .iter()
                .map(|response| response.replace("{addr}", &address.to_string()))
                .collect::<Vec<String>>();
            let handle = thread::spawn(move || serve_responses(&listener, &responses));

            Self {
                reference: format!("{address}/repo:tag"),
                handle: Some(handle),
            }
        }

        fn image(&self) -> ImageReference {
            ImageReference::parse(&self.reference)
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            join_server(self.handle.take());
        }
    }

    fn serve_responses(listener: &TcpListener, responses: &[String]) {
        for response in responses {
            serve_response(listener, response);
        }
    }

    fn serve_response(listener: &TcpListener, response: &str) {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut request = [0_u8; 2048];
        let _: usize = stream.read(&mut request).unwrap_or(0);
        stream
            .write_all(response.as_bytes())
            .expect("write test response");
        stream.flush().expect("flush test response");
    }

    fn join_server(handle: Option<thread::JoinHandle<()>>) {
        let Some(handle) = handle else {
            return;
        };
        handle.join().expect("join test server thread");
    }

    fn raw(status_line: &str, extra_headers: &[&str], body: &str) -> String {
        let mut response = format!("HTTP/1.1 {status_line}\r\n");
        for header in extra_headers {
            response.push_str(header);
            response.push_str("\r\n");
        }
        write!(
            response,
            "Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .expect("write test response");
        response
    }

    async fn authenticate_on(
        server: &TestServer,
        credentials: Option<Credentials>,
    ) -> Result<Option<String>> {
        let image = server.image();
        let client = build_client();

        authenticate(
            &client,
            image.scheme(),
            &image.registry,
            &image.name,
            Access::Pull,
            credentials.as_ref(),
        )
        .await
    }

    #[test]
    fn access_covers_pull_and_pull_push_actions() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(Access::Pull.actions(), "pull");
        assert_eq!(Access::PullPush.actions(), "pull,push");
    }

    #[test]
    fn basic_header_encodes_username_and_password() {
        // ARRANGE
        let credentials = Credentials::new("user", "pass");

        // ACT / ASSERT
        assert_eq!(credentials.basic_header(), "Basic dXNlcjpwYXNz");
    }

    #[test]
    fn from_parts_ignores_empty_values() {
        // ARRANGE / ACT / ASSERT
        assert!(Credentials::from_parts(Some("user"), Some("pass")).is_some());
        assert!(Credentials::from_parts(Some("user"), Some("")).is_none());
        assert!(Credentials::from_parts(Some(""), Some("pass")).is_none());
        assert!(Credentials::from_parts(None, None).is_none());
    }

    #[test]
    fn bearer_token_url_encodes_scope_and_service() {
        // ARRANGE
        let challenge =
            Challenge::parse(r#"Bearer realm="https://ghcr.io/token",service="ghcr.io""#)
                .expect("parse challenge");

        // ACT
        let url = bearer_token_url(&challenge, "muak-os/linux", Access::PullPush, "ghcr.io")
            .expect("build token url");

        // ASSERT
        assert_eq!(
            url,
            "https://ghcr.io/token?scope=repository%3Amuak-os%2Flinux%3Apull%2Cpush&service=ghcr.io"
        );
    }

    #[test]
    fn bearer_token_url_requires_realm() {
        // ARRANGE
        let challenge = Challenge::parse(r#"Bearer service="ghcr.io""#).expect("parse challenge");

        // ACT / ASSERT
        let error = bearer_token_url(&challenge, "repo", Access::Pull, "ghcr.io")
            .expect_err("missing realm should fail");
        assert!(matches!(error, KociError::AuthError { .. }));
    }

    #[test]
    fn parse_token_response_prefers_token_and_accepts_access_token() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(
            parse_token_response(br#"{"token":"abc"}"#, "ghcr.io").expect("parse token"),
            "Bearer abc"
        );
        assert_eq!(
            parse_token_response(br#"{"access_token":"def"}"#, "ghcr.io").expect("parse token"),
            "Bearer def"
        );
    }

    #[test]
    fn parse_token_response_rejects_empty_and_invalid_bodies() {
        // ARRANGE / ACT / ASSERT
        let error = parse_token_response(br#"{"expires_in":300}"#, "ghcr.io")
            .expect_err("missing token should fail");
        assert!(matches!(error, KociError::AuthError { .. }));

        let error =
            parse_token_response(b"not json", "ghcr.io").expect_err("invalid json should fail");
        assert!(matches!(error, KociError::AuthError { .. }));

        let error =
            parse_token_response(&[0xff, 0xfe], "ghcr.io").expect_err("non-utf8 body should fail");
        assert!(matches!(error, KociError::AuthError { .. }));
    }

    #[tokio::test]
    async fn authenticate_returns_none_when_ping_succeeds() {
        // ARRANGE
        let server = TestServer::spawn(&[&raw("200 OK", &[], "")]);

        // ACT
        let auth = authenticate_on(&server, None)
            .await
            .expect("authenticate should succeed");

        // ASSERT
        assert_eq!(auth, None);
    }

    #[tokio::test]
    async fn authenticate_tolerates_unexpected_ping_status() {
        // ARRANGE
        let server = TestServer::spawn(&[&raw("404 Not Found", &[], "")]);

        // ACT
        let auth = authenticate_on(&server, None)
            .await
            .expect("authenticate should succeed");

        // ASSERT
        assert_eq!(auth, None);
    }

    #[tokio::test]
    async fn authenticate_fetches_anonymous_bearer_token_from_challenge() {
        // ARRANGE
        let server = TestServer::spawn(&[
            &raw("401 Unauthorized", &[BEARER_CHALLENGE_TEMPLATE], ""),
            &raw("200 OK", &[], r#"{"token":"abc123"}"#),
        ]);

        // ACT
        let auth = authenticate_on(&server, None)
            .await
            .expect("authenticate should succeed");

        // ASSERT
        assert_eq!(auth.as_deref(), Some("Bearer abc123"));
    }

    #[tokio::test]
    async fn authenticate_answers_basic_challenge_with_credentials() {
        // ARRANGE
        let server = TestServer::spawn(&[&raw("401 Unauthorized", &[BASIC_CHALLENGE], "")]);

        // ACT
        let auth = authenticate_on(&server, Some(Credentials::new("user", "pass")))
            .await
            .expect("authenticate should succeed");

        // ASSERT
        assert_eq!(auth.as_deref(), Some("Basic dXNlcjpwYXNz"));
    }

    #[tokio::test]
    async fn authenticate_reports_missing_credentials_for_basic_challenge() {
        // ARRANGE
        let server = TestServer::spawn(&[&raw("401 Unauthorized", &[BASIC_CHALLENGE], "")]);

        // ACT
        let error = authenticate_on(&server, None)
            .await
            .expect_err("missing credentials should fail");

        // ASSERT
        assert!(matches!(error, KociError::AuthError { .. }));
    }

    #[tokio::test]
    async fn authenticate_reports_unparsable_challenge() {
        // ARRANGE
        let server = TestServer::spawn(&[&raw("401 Unauthorized", &[], "")]);

        // ACT
        let error = authenticate_on(&server, None)
            .await
            .expect_err("missing challenge should fail");

        // ASSERT
        assert!(matches!(error, KociError::AuthError { .. }));
    }

    #[tokio::test]
    async fn authenticate_reports_token_endpoint_failures() {
        // ARRANGE
        let server = TestServer::spawn(&[
            &raw("401 Unauthorized", &[BEARER_CHALLENGE_TEMPLATE], ""),
            &raw("500 Internal Server Error", &[], ""),
        ]);

        // ACT
        let error = authenticate_on(&server, None)
            .await
            .expect_err("token endpoint failure should fail");

        // ASSERT
        assert!(
            matches!(error, KociError::AuthError { details, .. } if details.contains("failed to fetch bearer token"))
        );
    }
}
