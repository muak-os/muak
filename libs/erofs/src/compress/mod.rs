//! Destsize-bounded zstd compression for EROFS, producing per-pcluster streams.

mod config;
mod fit;
mod model;
mod zstd;

pub type Compression = config::Compression;
pub const DEFAULT_ZSTD_COMPRESSION_LEVEL: i32 = config::DEFAULT_ZSTD_COMPRESSION_LEVEL;
pub type CompressedFile = model::CompressedFile;

use crate::error::{ErofsError, Result};

/// Compress file data into multiple destsize-bounded pclusters.
pub fn compress_file(data: &[u8], compression_level: i32) -> Result<Option<CompressedFile>> {
    fit::compress_file(data, compression_level)
}

pub(crate) fn validate_compression_level(level: i32) -> Result<i32> {
    let range = ::zstd::compression_level_range();

    if level == 0 || range.contains(&level) {
        Ok(level)
    } else {
        Err(ErofsError::InvalidCompressionLevel {
            level,
            min: *range.start(),
            max: *range.end(),
        })
    }
}

pub(crate) fn has_representable_compact_indexes(cf: &CompressedFile) -> bool {
    fit::has_representable_compact_indexes(cf)
}

/// Number of logical clusters (input 4KB blocks).
pub fn lcluster_count(cf: &CompressedFile) -> u32 {
    model::lcluster_count(cf)
}

/// Total number of physical 4KB blocks needed across all pclusters.
pub fn pcluster_blocks(cf: &CompressedFile) -> u32 {
    model::pcluster_blocks(cf)
}
