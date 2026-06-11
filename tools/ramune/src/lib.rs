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

/// An archive entry to include in the CPIO archive.
pub type Entry<'a> = archive::Entry<'a>;

/// Default zstd compression level.
pub const DEFAULT_ZSTD_COMPRESSION_LEVEL: i32 = 6;

/// Writes a zstd-compressed CPIO archive containing the given entries.
///
/// # Errors
///
/// Returns an error when validation fails, an entry exceeds CPIO limits,
/// reading an entry fails, or zstd compression fails.
pub fn archive<W: std::io::Write>(
    entries: &mut [Entry],
    compression_level: i32,
    writer: &mut W,
) -> error::Result<()> {
    archive::archive(entries, compression_level, writer)
}
