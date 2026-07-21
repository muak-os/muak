//! Interact with TPM2 via `/dev/tpmrm0`.

#![warn(missing_docs)]

mod auth;
pub mod blob;
mod buffer;
mod commands;
pub mod device;
mod error;
mod handles;
pub mod operations;
pub mod pcr;
mod response;
