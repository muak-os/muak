//! OCI protocol helpers: authentication, HTTP, manifests, layers, signing, and verification.

/// Accepted media types for OCI manifest requests.
const OCI_MANIFEST_ACCEPT_HEADERS: &[&str] = &[
    "application/vnd.oci.image.manifest.v1+json",
    "application/vnd.docker.distribution.manifest.v2+json",
    "application/vnd.oci.image.index.v1+json",
    "application/vnd.docker.distribution.manifest.list.v2+json",
];

/// HTTP User-Agent header value sent to OCI registries.
const USER_AGENT: &str = "muak-imager/0.1";

mod auth;
mod http;
mod layer;
mod manifest;
pub(crate) mod sign;
pub(crate) mod verify;

pub(crate) mod remote;
