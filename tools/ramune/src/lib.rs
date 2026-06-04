//! Ramune: initramfs builder for creating base images and appending extra files.

pub mod builder;
#[cfg(feature = "cli")]
pub mod cli;
mod compress;
mod cpio;
mod erofs;
pub mod error;
pub mod extender;
mod extra;

pub type CreateConfig<'a> = builder::CreateConfig<'a>;
pub type RamuneError = error::RamuneError;
pub type ExtendConfig<'a> = extender::ExtendConfig<'a>;
pub type ExtraFile<'a> = extender::ExtraFile<'a>;

pub const DEFAULT_ZSTD_COMPRESSION_LEVEL: i32 = 6;

/// Creates a base initramfs image from an init binary and rootfs directory.
///
/// # Errors
///
/// Returns an error when reading inputs, building the staged rootfs, compressing the archive,
/// or writing the output image fails.
pub fn create(config: &CreateConfig<'_>, output: &std::path::Path) -> error::Result<()> {
    builder::create(config, output)
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
