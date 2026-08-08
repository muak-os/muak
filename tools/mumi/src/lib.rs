//! Mumi: seals file sets into compressed EROFS images.

#![warn(missing_docs)]

#[cfg(feature = "cli")]
pub mod cli;
pub mod error;
pub mod image;
pub mod payload;
pub mod rootfs;

/// Default zstd compression level for EROFS images.
pub const DEFAULT_ZSTD_COMPRESSION_LEVEL: i32 = erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL;
