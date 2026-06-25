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

/// An archive entry with a readable payload stream attached.
pub type EntryStream<'a> = archive::EntryStream<'a>;

/// Default zstd compression level.
pub const DEFAULT_ZSTD_COMPRESSION_LEVEL: i32 = 6;

/// Writes a zstd-compressed CPIO archive containing the given entries.
///
/// # Errors
///
/// Returns an error when validation fails, an entry exceeds CPIO limits,
/// reading an entry fails, or zstd compression fails.
pub fn archive<W: std::io::Write>(
    streams: &mut [EntryStream],
    compression_level: i32,
    writer: &mut W,
) -> error::Result<()> {
    archive::archive(streams, compression_level, writer)
}

/// Writes a raw CPIO newc archive (no zstd compression) containing the given
/// entries and returns the exact number of bytes written.
///
/// # Errors
///
/// Returns an error when validation fails, an entry exceeds CPIO limits,
/// or reading/writing an entry fails.
pub fn raw<W: std::io::Write>(streams: &mut [EntryStream], writer: &mut W) -> error::Result<u64> {
    archive::raw(streams, writer)
}

/// Returns the exact byte length of a raw CPIO newc archive (no zstd compression)
/// containing the given entries. Pure computation — no I/O is performed.
#[must_use]
pub fn raw_size(entries: &[Entry]) -> u64 {
    archive::raw_size(entries)
}
