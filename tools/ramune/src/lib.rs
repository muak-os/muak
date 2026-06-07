//! Ramune: initramfs builder for creating base images and appending extra files.

#![warn(missing_docs)]

pub mod builder;
#[cfg(feature = "cli")]
pub mod cli;
mod compress;
mod cpio;
mod erofs;
pub mod error;
pub mod extender;
mod extra;

/// Configuration for creating a base initramfs.
pub type CreateConfig<'a> = builder::CreateConfig<'a>;
/// Configuration for extending an existing initramfs.
pub type ExtendConfig<'a> = extender::ExtendConfig<'a>;
/// An extra file to append to an initramfs archive.
pub type ExtraFile<'a> = extender::ExtraFile<'a>;

/// Default zstd compression level.
pub const DEFAULT_ZSTD_COMPRESSION_LEVEL: i32 = 6;

/// Creates a base initramfs image from an init binary and rootfs directory,
/// writing the compressed archive into `writer`.
///
/// # Errors
///
/// Returns an error when reading inputs, building the staged rootfs, compressing the archive,
/// or writing to the output sink fails.
pub fn create<W: std::io::Write>(
    config: &CreateConfig<'_>,
    writer: &mut W,
) -> error::Result<()> {
    builder::create(config, writer)
}

/// Extends an initramfs image by appending a compressed archive of extra files.
///
/// # Errors
///
/// Returns an error when validation fails, reading the base image or extra files fails,
/// compressing the appended archive fails, or writing the output image fails.
pub async fn extend(config: &ExtendConfig<'_>, output: &std::path::Path) -> error::Result<()> {
    extender::extend(config, output).await
}
