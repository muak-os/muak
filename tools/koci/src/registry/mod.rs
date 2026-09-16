pub(crate) mod auth;
pub(crate) mod challenge;
pub(crate) mod http;
pub(crate) mod manifest;
pub(crate) mod redirect;
pub(crate) mod session;

/// HTTP User-Agent header value sent to OCI registries.
pub(crate) const USER_AGENT: &str = "muak-koci/0.1";
