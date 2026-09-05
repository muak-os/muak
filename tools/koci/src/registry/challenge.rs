//! Parsing of `WWW-Authenticate` challenges (RFC 7235).

/// A single `WWW-Authenticate` challenge: an auth scheme plus parameters.
#[derive(Clone)]
pub(crate) struct Challenge {
    /// Auth scheme, lowercased (`bearer`, `basic`, ...).
    pub(crate) scheme: String,
    /// Challenge parameters such as `realm`, `service`, and `scope`.
    params: Vec<(String, String)>,
}

impl Challenge {
    /// Parse one `WWW-Authenticate` header value into a [`Challenge`].
    pub(crate) fn parse(value: &str) -> Option<Self> {
        let (scheme, params) = value.trim().split_once(char::is_whitespace)?;
        let scheme = scheme.trim().to_ascii_lowercase();
        if scheme.is_empty() {
            return None;
        }

        let params: Vec<(String, String)> = params.split(',').filter_map(parse_param).collect();

        Some(Self { scheme, params })
    }

    /// Look up a challenge parameter by key.
    pub(crate) fn param(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|param| param.0 == key)
            .map(|param| param.1.as_str())
    }
}

/// Parse one `key="value"` challenge parameter.
fn parse_param(pair: &str) -> Option<(String, String)> {
    let (key, value) = pair.split_once('=')?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }

    Some((key.to_owned(), value.trim().trim_matches('"').to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_bearer_parameters() {
        // ARRANGE
        let header = r#"Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:muak-os/linux:pull""#;

        // ACT
        let challenge = Challenge::parse(header).expect("parse bearer challenge");

        // ASSERT
        assert_eq!(challenge.scheme, "bearer");
        assert_eq!(challenge.param("realm"), Some("https://ghcr.io/token"));
        assert_eq!(challenge.param("service"), Some("ghcr.io"));
        assert_eq!(
            challenge.param("scope"),
            Some("repository:muak-os/linux:pull")
        );
    }

    #[test]
    fn parse_is_case_insensitive_and_tolerates_whitespace() {
        // ARRANGE
        let header = r#"BEARER  realm="https://example.test/token" , service="example.test""#;

        // ACT
        let challenge = Challenge::parse(header).expect("parse challenge");

        // ASSERT
        assert_eq!(challenge.scheme, "bearer");
        assert_eq!(challenge.param("realm"), Some("https://example.test/token"));
        assert_eq!(challenge.param("service"), Some("example.test"));
    }

    #[test]
    fn parse_reads_unquoted_parameters() {
        // ARRANGE
        let header = "Bearer error=insufficient_scope,realm=\"https://example.test\"";

        // ACT
        let challenge = Challenge::parse(header).expect("parse challenge");

        // ASSERT
        assert_eq!(challenge.param("error"), Some("insufficient_scope"));
        assert_eq!(challenge.param("realm"), Some("https://example.test"));
    }

    #[test]
    fn parse_rejects_values_without_scheme() {
        // ARRANGE / ACT / ASSERT
        assert!(Challenge::parse("realm=\"https://example.test\"").is_none());
        assert!(Challenge::parse("   ").is_none());
    }

    #[test]
    fn param_returns_none_for_missing_keys() {
        // ARRANGE
        let challenge =
            Challenge::parse("Basic realm=\"https://example.test\"").expect("parse challenge");

        // ACT / ASSERT
        assert_eq!(challenge.scheme, "basic");
        assert_eq!(challenge.param("service"), None);
    }
}
