//! Miso - Packages a Unified Kernel Image into a bootable image.

#![warn(missing_docs)]

#[cfg(feature = "cli")]
pub mod cli;
pub mod error;
pub mod iso;
pub mod raw;

use std::io::Write;

use esp::EspSpec;

use crate::error::{MisoError, Result};

/// Builds a bootable ISO 9660 image from an `esp::EspSpec`.
///
/// # Errors
///
/// Returns an error if ESP construction fails or writing the ISO image fails.
pub fn build_iso<W: Write>(spec: &mut EspSpec, out: &mut W) -> Result<()> {
    let esp_size = esp::compute_fat_size(&spec.metas().collect::<Vec<_>>())?;
    iso::write(out, esp_size, |w| {
        esp::build(spec.files_mut(), w)?;
        Ok(())
    })
}

/// Builds a raw GPT disk image from an `esp::EspSpec` into any `Write` sink.
///
/// # Errors
///
/// Returns an error if ESP construction fails, compression level validation fails, raw image creation fails, or output writing/compression fails.
pub fn build_raw<W: Write>(
    spec: &mut EspSpec,
    out: &mut W,
    compression_level: Option<i32>,
) -> Result<()> {
    let esp_size = esp::compute_fat_size(&spec.metas().collect::<Vec<_>>())?;

    if let Some(level) = compression_level {
        let level = validate_compression_level(level)?;
        let mut encoder = zstd::Encoder::new(out, level).map_err(MisoError::ZstdInit)?;
        raw::write(&mut encoder, esp_size, |w| {
            esp::build(spec.files_mut(), w)?;
            Ok(())
        })?;
        encoder.finish().map_err(MisoError::Compression)?;
    } else {
        raw::write(out, esp_size, |w| {
            esp::build(spec.files_mut(), w)?;
            Ok(())
        })?;
    }

    Ok(())
}

fn validate_compression_level(level: i32) -> Result<i32> {
    let range = zstd::compression_level_range();

    if level == 0 || range.contains(&level) {
        Ok(level)
    } else {
        Err(MisoError::InvalidCompressionLevel {
            level,
            min: *range.start(),
            max: *range.end(),
        })
    }
}
