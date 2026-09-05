pub(crate) mod auth;
pub(crate) mod http;
pub(crate) mod session;

/// Accepted media types for OCI manifest requests.
pub(crate) const OCI_MANIFEST_ACCEPT_HEADERS: &[&str] = &[
    "application/vnd.oci.image.manifest.v1+json",
    "application/vnd.docker.distribution.manifest.v2+json",
    "application/vnd.oci.image.index.v1+json",
    "application/vnd.docker.distribution.manifest.list.v2+json",
];

/// HTTP User-Agent header value sent to OCI registries.
pub(crate) const USER_AGENT: &str = "muak-koci/0.1";
