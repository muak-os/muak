//! Error types for the distribution-spec client.

use core::time::Duration;

use thiserror::Error;

/// Error type for registry transport and authentication operations.
#[expect(
    clippy::module_name_repetitions,
    reason = "The public error type name intentionally includes the crate name"
)]
#[derive(Error, Debug)]
pub enum ClientError {
    /// Failed to download from the registry.
    #[error("Failed to download image: {0}")]
    Download(String),

    /// Failed to push to the registry.
    #[error("Failed to push image: {0}")]
    Push(String),

    /// Registry rejected the authentication attempt.
    #[error("Registry authentication failed for {registry}: {details}")]
    Auth {
        /// Registry host the authentication failed against.
        registry: String,
        /// Failure details from the registry or the auth challenge.
        details: String,
    },

    /// A network request failed.
    #[error("Network error: {0}")]
    Network(String),

    /// The registry answered a non-success status for a URL.
    #[error("HTTP {status} for URL: {url}")]
    Status {
        /// The non-success HTTP status code.
        status: u16,
        /// The requested URL.
        url: String,
    },
}

impl ClientError {
    /// Builds a network failure naming the URL and the full source chain.
    pub(crate) fn network(
        url: &str,
        context: &str,
        cause: &(dyn std::error::Error + 'static),
    ) -> Self {
        Self::Network(format!("{context} for URL: {url}: {}", chain(cause)))
    }

    /// Builds a response body timeout naming the URL and the deadline.
    pub(crate) fn body_timeout(
        url: &str,
        limit: Duration,
        cause: &(dyn std::error::Error + 'static),
    ) -> Self {
        Self::network(
            url,
            &format!("HTTP response body timed out after {limit:?}"),
            cause,
        )
    }
}

/// Result type alias for distribution-spec client operations.
pub type Result<T> = core::result::Result<T, ClientError>;

fn chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut rendered = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        rendered.push_str(": ");
        rendered.push_str(&cause.to_string());
        source = cause.source();
    }

    rendered
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use super::ClientError;

    const CONSTRUCTORS: [(&str, &str); 6] = [
        ("auth.rs", include_str!("auth.rs")),
        ("http.rs", include_str!("http.rs")),
        ("manifest.rs", include_str!("manifest.rs")),
        ("redirect.rs", include_str!("redirect.rs")),
        ("request.rs", include_str!("request.rs")),
        ("tags.rs", include_str!("tags.rs")),
    ];

    #[derive(Debug)]
    struct Outer {
        inner: Inner,
    }

    impl core::fmt::Display for Outer {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("client error (Connect)")
        }
    }

    impl std::error::Error for Outer {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.inner)
        }
    }

    #[derive(Debug)]
    struct Inner(&'static str);

    impl core::fmt::Display for Inner {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str(self.0)
        }
    }

    impl std::error::Error for Inner {}

    fn production(source: &str) -> &str {
        source.split("#[cfg(test)]").next().unwrap_or(source)
    }

    #[test]
    fn network_errors_are_built_through_the_constructor() {
        // ARRANGE
        let offenders: Vec<&str> = CONSTRUCTORS
            .iter()
            .filter(|entry| production(entry.1).contains("ClientError::Network("))
            .map(|entry| entry.0)
            .collect();

        // ACT
        let constructed = production(include_str!("error.rs"))
            .matches("Self::Network(")
            .count();

        // ASSERT
        assert!(
            offenders.is_empty(),
            "these modules must use ClientError::network: {offenders:?}"
        );
        assert_eq!(constructed, 1, "the variant is built once, in error.rs");
    }

    #[test]
    fn network_error_names_the_url_and_the_cause() {
        // ARRANGE
        let url = "https://registry.example/v2/foo/blobs/sha256:abc";

        // ACT
        let message = ClientError::network(
            url,
            "HTTP request failed",
            &std::io::Error::other("connection refused"),
        )
        .to_string();

        // ASSERT
        assert_eq!(
            message,
            format!("Network error: HTTP request failed for URL: {url}: connection refused")
        );
    }

    #[test]
    fn body_timeout_names_the_url_and_the_deadline() {
        // ARRANGE
        let url = "https://registry.example/v2/foo/blobs/sha256:abc";

        // ACT
        let message = ClientError::body_timeout(
            url,
            Duration::from_secs(7),
            &std::io::Error::other("elapsed"),
        )
        .to_string();

        // ASSERT
        assert!(
            message.contains(&format!("for URL: {url}")),
            "body timeout must name the URL: {message}"
        );
        assert!(
            message.contains("7s"),
            "body timeout must name the deadline: {message}"
        );
    }

    #[test]
    fn network_error_renders_the_source_chain() {
        // ARRANGE
        let url = "https://ghcr.io/v2/";
        let cause = Outer {
            inner: Inner("invalid peer certificate"),
        };

        // ACT
        let message = ClientError::network(url, "HTTP request failed", &cause).to_string();

        // ASSERT
        assert!(
            message.contains("client error (Connect): invalid peer certificate"),
            "network error must render the whole chain: {message}"
        );
    }
}
