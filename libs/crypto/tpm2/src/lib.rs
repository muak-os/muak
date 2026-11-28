//! Interact with TPM2 through a sans-io core with a `/dev/tpmrm0` default transport.

#![warn(missing_docs)]

mod auth;
pub mod blob;
mod buffer;
mod commands;
pub mod device;
pub mod error;
mod handles;
pub mod operations;
pub mod pcr;
mod response;
mod tags;
pub mod transport;
