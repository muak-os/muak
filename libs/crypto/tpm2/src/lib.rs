//! Sans-io, allocation-free TPM2 core for PCR-sealed blobs, with an optional Linux transport behind the `linux` feature, enabled by default.

#![warn(missing_docs)]

#[cfg(all(feature = "linux", not(target_os = "linux")))]
compile_error!("the `linux` tpm2 feature requires a Linux target");

mod auth;
pub mod blob;
mod buffer;
mod commands;
pub mod error;
mod handles;
pub mod operations;
pub mod pcr;
mod response;
mod tags;
pub mod transport;

#[cfg(all(feature = "linux", target_os = "linux"))]
pub mod linux;
