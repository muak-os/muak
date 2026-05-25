//! Shared zstd compression helpers for ramune image builders.

use std::io::Write;

use crate::error::{RamuneError, Result};

const ZSTD_WORKERS: u32 = 8;

/// Validates that the provided compression level is within the valid range.
pub(crate) fn validate_level(compression_level: i32) -> Result<i32> {
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

/// Creates a zstd encoder with the specified compression level and number of workers.
pub(crate) fn encoder<W: Write>(
    writer: W,
    compression_level: i32,
) -> Result<zstd::Encoder<'static, W>> {
    let compression_level = validate_level(compression_level)?;
    let mut encoder =
        zstd::Encoder::new(writer, compression_level).map_err(RamuneError::ZstdInitError)?;
    encoder
        .multithread(ZSTD_WORKERS)
        .map_err(RamuneError::ZstdInitError)?;
    Ok(encoder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoder_accepts_valid_level() {
        // ARRANGE / ACT
        let result = encoder(Vec::new(), crate::DEFAULT_ZSTD_COMPRESSION_LEVEL);

        // ASSERT
        assert!(result.is_ok());
    }

    #[test]
    fn encoder_rejects_invalid_level() {
        // ARRANGE / ACT
        let result = encoder(Vec::new(), i32::MAX);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::InvalidCompressionLevel { .. }))
        );
    }

    #[test]
    fn encoder_writes_valid_zstd_stream() {
        // ARRANGE
        let mut encoder = encoder(Vec::new(), crate::DEFAULT_ZSTD_COMPRESSION_LEVEL)
            .expect("encoder creation should succeed");

        // ACT
        encoder.write_all(b"payload").expect("write payload");

        let compressed = encoder.finish().expect("finish compression");
        let decompressed = zstd::decode_all(compressed.as_slice()).expect("decode zstd stream");

        // ASSERT
        assert_eq!(decompressed, b"payload");
    }

    #[test]
    fn validate_level_accepts_valid_level() {
        // ARRANGE / ACT
        let result = validate_level(crate::DEFAULT_ZSTD_COMPRESSION_LEVEL);

        // ASSERT
        assert_eq!(
            result.expect("valid compression level"),
            crate::DEFAULT_ZSTD_COMPRESSION_LEVEL
        );
    }

    #[test]
    fn validate_level_accepts_zero() {
        // ARRANGE / ACT
        let result = validate_level(0);

        // ASSERT
        assert_eq!(result.expect("zero compression level"), 0);
    }

    #[test]
    fn validate_level_rejects_invalid_level() {
        // ARRANGE / ACT
        let result = validate_level(i32::MAX);

        // ASSERT
        assert!(
            result
                .as_ref()
                .is_err_and(|error| matches!(error, RamuneError::InvalidCompressionLevel { .. }))
        );
    }
}
