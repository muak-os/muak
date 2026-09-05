pub(crate) mod auth;
pub(crate) mod challenge;
pub(crate) mod http;
pub(crate) mod session;

/// Media type of an OCI image manifest.
pub(crate) const OCI_MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
/// Media type of a Docker schema 2 image manifest.
pub(crate) const DOCKER_MANIFEST_MEDIA_TYPE: &str =
    "application/vnd.docker.distribution.manifest.v2+json";
/// Media type of an OCI image index.
pub(crate) const OCI_IMAGE_INDEX_MEDIA_TYPE: &str = "application/vnd.oci.image.index.v1+json";
/// Media type of a Docker schema 2 manifest list.
pub(crate) const DOCKER_MANIFEST_LIST_MEDIA_TYPE: &str =
    "application/vnd.docker.distribution.manifest.list.v2+json";

/// Accepted media types for OCI manifest requests.
pub(crate) const OCI_MANIFEST_ACCEPT_HEADERS: &[&str] = &[
    OCI_MANIFEST_MEDIA_TYPE,
    DOCKER_MANIFEST_MEDIA_TYPE,
    OCI_IMAGE_INDEX_MEDIA_TYPE,
    DOCKER_MANIFEST_LIST_MEDIA_TYPE,
];

/// HTTP User-Agent header value sent to OCI registries.
pub(crate) const USER_AGENT: &str = "muak-koci/0.1";
