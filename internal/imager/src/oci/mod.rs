const OCI_MANIFEST_ACCEPT_HEADERS: &[&str] = &[
    "application/vnd.oci.image.manifest.v1+json",
    "application/vnd.docker.distribution.manifest.v2+json",
    "application/vnd.oci.image.index.v1+json",
    "application/vnd.docker.distribution.manifest.list.v2+json",
];

const USER_AGENT: &str = "muak-imager/0.1";

mod auth;
mod http;
mod layer;
mod manifest;

pub mod local;
pub mod remote;
