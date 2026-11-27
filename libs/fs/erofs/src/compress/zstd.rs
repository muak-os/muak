//! Thin zstd context setup helpers.

use zstd::zstd_safe::{CCtx, CParameter};

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

#[cfg(test)]
mod tests {
    use super::new_cctx;
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
}
