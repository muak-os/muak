//! RFC 791/8200/768 framing and RFC 1071 checksums.

#![warn(missing_docs)]

pub mod checksum;
pub mod error;
#[cfg(feature = "v4")]
pub mod v4;
#[cfg(feature = "v6")]
pub mod v6;
