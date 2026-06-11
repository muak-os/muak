//! Ramune: initramfs builder for creating base images and compressed append tails.

#![warn(missing_docs)]

pub mod builder;
#[cfg(feature = "cli")]
pub mod cli;
mod compress;
mod cpio;
mod erofs;
pub mod error;
pub mod extender;

/// Configuration for creating a base initramfs.
pub type CreateConfig<'a> = builder::CreateConfig<'a>;
/// Configuration for creating a compressed append tail.
pub type TailConfig<'a> = extender::TailConfig<'a>;
/// An archive entry to append to an initramfs tail.
pub type AppendEntry<'a> = extender::AppendEntry<'a>;

/// Default zstd compression level.
pub const DEFAULT_ZSTD_COMPRESSION_LEVEL: i32 = 6;

/// Creates a base initramfs image from an init binary and rootfs directory,
/// writing the compressed archive into `writer`.
///
/// # Errors
///
/// Returns an error when reading inputs, building the staged rootfs, compressing the archive,
/// or writing to the output sink fails.
pub fn create<W: std::io::Write>(config: &CreateConfig<'_>, writer: &mut W) -> error::Result<()> {
    builder::create(config, writer)
}

/// Builds a compressed append tail containing the configured archive entries.
///
/// # Errors
///
/// Returns an error when validation fails, streaming an entry fails,
/// or compressing the appended archive fails.
pub fn build_tail(config: &mut TailConfig<'_>) -> error::Result<Vec<u8>> {
    extender::build_tail(config)
}
