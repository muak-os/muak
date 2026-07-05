//! Ramune: initramfs builder for creating base images and compressed append tails.

#![warn(missing_docs)]

pub mod archive;
#[cfg(feature = "cli")]
pub mod cli;
mod compress;
mod cpio;
mod erofs;
pub mod error;
pub mod rootfs;

/// An archive entry to include in a CPIO archive.
pub type Entry = archive::Entry;

/// Default zstd compression level.
pub const DEFAULT_ZSTD_COMPRESSION_LEVEL: i32 = 6;
