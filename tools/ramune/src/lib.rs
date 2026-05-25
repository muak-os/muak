//! Ramune: initramfs builder for creating base images and appending extensions.

pub mod builder;
#[cfg(feature = "cli")]
pub mod cli;
mod cpio;
mod erofs;
pub mod error;
pub mod extender;
mod extension;

pub type CreateConfig<'a> = builder::CreateConfig<'a>;
pub type RamuneError = error::RamuneError;
pub type ExtendConfig<'a> = extender::ExtendConfig<'a>;

pub const DEFAULT_ZSTD_COMPRESSION_LEVEL: i32 = 6;
pub const EROFS_DEFAULT_ZSTD_COMPRESSION_LEVEL: i32 = ::erofs::DEFAULT_ZSTD_COMPRESSION_LEVEL;

/// Creates a base initramfs image from an init binary and rootfs directory.
///
/// # Errors
///
/// Returns an error when reading inputs, building the staged rootfs, compressing the archive,
/// or writing the output image fails.
pub fn create(config: &CreateConfig<'_>, output: &std::path::Path) -> error::Result<()> {
    builder::create(config, output)
}

/// Extends an initramfs image with an appended compressed EROFS extensions archive.
///
/// # Errors
///
/// Returns an error when reading the base image, processing extensions, compressing the
/// appended archive, or writing the output image fails.
pub async fn extend(config: &ExtendConfig<'_>, output: &std::path::Path) -> error::Result<()> {
    extender::extend(config, output).await
}

pub(crate) fn validate_compression_level(compression_level: i32) -> error::Result<i32> {
    let range = zstd::compression_level_range();

    if compression_level == 0 || range.contains(&compression_level) {
        Ok(compression_level)
    } else {
        Err(error::RamuneError::InvalidCompressionLevel {
            level: compression_level,
            min: *range.start(),
            max: *range.end(),
        })
    }
}
