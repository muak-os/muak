//! Thin zstd context setup and whole-input compression helpers.

use zstd::zstd_safe::{CCtx, CParameter, compress_bound, get_error_name};

use super::validate_compression_level;
use crate::error::{ErofsError, Result};

pub(super) const ZSTD_WINDOW_LOG: u32 = 15;
pub(super) const PCLUSTER_SIZE: usize = 4096;

pub(super) fn new_cctx(compression_level: i32) -> Result<CCtx<'static>> {
    let compression_level = validate_compression_level(compression_level)?;
    let mut cctx = CCtx::create();
    cctx.set_parameter(CParameter::CompressionLevel(compression_level))
        .map_err(compression_error)?;
    cctx.set_parameter(CParameter::WindowLog(ZSTD_WINDOW_LOG))
        .map_err(compression_error)?;
    Ok(cctx)
}

pub(super) fn compression_error(code: usize) -> ErofsError {
    ErofsError::Compression {
        detail: format!("zstd error code: {code}"),
    }
}

pub(super) fn compress_whole_input(cctx: &mut CCtx<'_>, src: &[u8]) -> Result<Vec<u8>> {
    let upper = compress_bound(src.len());
    let mut dst = vec![0_u8; upper];
    let written = cctx.compress2(&mut dst, src).map_err(compression_error)?;
    dst.truncate(written);
    Ok(dst)
}

pub(super) fn error_name(code: usize) -> &'static str {
    get_error_name(code)
}

#[cfg(test)]
mod tests {
    use super::{compress_whole_input, new_cctx};
    use crate::error::ErofsError;

    #[test]
    fn compress_invalid_level_errors() {
        // ARRANGE & ACT
        let result = new_cctx(i32::MAX);
        // ASSERT
        assert!(matches!(
            result,
            Err(ErofsError::InvalidCompressionLevel { .. })
        ));
    }

    #[test]
    fn compress_whole_input_round_trips() {
        // ARRANGE
        let mut cctx = new_cctx(3).expect("cctx");
        let data = vec![0_u8; 8192];

        let compressed = compress_whole_input(&mut cctx, &data).expect("compress");
        let decompressed = zstd::bulk::decompress(&compressed, data.len()).expect("decompress");

        // ACT
        // ASSERT
        assert_eq!(decompressed, data);
    }
}
