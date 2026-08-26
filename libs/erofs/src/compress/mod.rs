//! Deterministic single-pass zstd compression producing layout-only pclusters.

mod chunk;
mod config;
mod model;
mod zstd;

pub type Compression = config::Compression;
pub const DEFAULT_ZSTD_COMPRESSION_LEVEL: i32 = config::DEFAULT_ZSTD_COMPRESSION_LEVEL;
pub type CompressedLayout = model::CompressedLayout;
pub(super) const CHUNK_MAX: usize = chunk::CHUNK_MAX;

use std::io::Read;

use crate::error::{ErofsError, Result};

/// Measure file data into layout-only pclusters without retaining bytes (pass 1).
pub(crate) fn compress_file(
    reader: &mut dyn Read,
    size: usize,
    path: &str,
    compression_level: i32,
) -> Result<Option<CompressedLayout>> {
    chunk::compress_file(reader, size, path, compression_level)
}

/// Re-compress one recorded pcluster in a single shot (pass 2 emit).
pub(crate) fn recompress_pcluster(
    level: i32,
    reader: &mut dyn Read,
    input_len: usize,
    src: &mut [u8],
    dst: &mut [u8],
) -> Result<usize> {
    chunk::recompress_pcluster(level, reader, input_len, src, dst)
}

pub(crate) fn has_representable_compact_indexes(cf: &CompressedLayout) -> bool {
    chunk::has_representable_compact_indexes(cf)
}

/// Number of logical clusters (input 4KB blocks).
pub fn lcluster_count(cf: &CompressedLayout) -> u32 {
    model::lcluster_count(cf)
}

/// Total number of physical 4KB blocks needed across all pclusters.
pub fn pcluster_blocks(cf: &CompressedLayout) -> u32 {
    model::pcluster_blocks(cf)
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

#[cfg(test)]
pub(crate) fn measure_slice(data: &[u8], level: i32) -> Result<Option<CompressedLayout>> {
    let mut cursor = std::io::Cursor::new(data);
    chunk::compress_file(&mut cursor, data.len(), "/test", level)
}
