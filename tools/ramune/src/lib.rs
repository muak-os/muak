//! Ramune: initramfs builder for creating base images and appending extensions.

#[cfg(feature = "cli")]
pub mod cli;
mod cpio;
mod create;
mod erofs;
mod error;
mod extend;
mod extension;

pub use create::{CreateConfig, create};
pub use error::{RamuneError, Result};
pub use extend::{ExtendConfig, extend};

pub const DEFAULT_ZSTD_COMPRESSION_LEVEL: i32 = 3;

pub(crate) fn validate_compression_level(compression_level: i32) -> Result<i32> {
    let range = zstd::compression_level_range();

    if compression_level == 0 || range.contains(&compression_level) {
        Ok(compression_level)
    } else {
        Err(RamuneError::InvalidCompressionLevel {
            level: compression_level,
            min: *range.start(),
            max: *range.end(),
        })
    }
}
