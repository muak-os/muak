//! No std, sans-io, allocation-free RFC 791/768 framing, RFC 8200 IPv6 upper-layer and RFC 1071 checksums.

#![no_std]
#![warn(missing_docs)]

pub mod checksum;
pub mod error;
#[cfg(feature = "v4")]
pub mod v4;
#[cfg(feature = "v6")]
pub mod v6;
