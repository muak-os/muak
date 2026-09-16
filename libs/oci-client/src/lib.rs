//! OCI distribution-spec registry client: authentication, blob, and manifest operations.

#![warn(missing_docs)]

pub mod auth;
pub mod blob;
pub mod challenge;
pub mod client;
pub mod error;
pub mod http;
pub mod manifest;
pub mod redirect;
