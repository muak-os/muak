//! Miso - Packages a Unified Kernel Image into a bootable image.

#![warn(missing_docs)]

#[cfg(feature = "cli")]
pub mod cli;
pub mod error;
pub mod iso;
pub mod raw;

use std::io::{Read, Write};

use esp::builder::{Builder, Layout};

use crate::error::{MisoError, Result};

/// Builds a bootable ISO 9660 image from a `Layout`.
///
/// # Errors
///
/// Returns an error if ESP construction fails or writing the ISO image fails.
pub fn build_iso<'data, 'ctx, W: Write>(
    layout: &'ctx Layout<'data>,
    readers: &mut [&'data mut (dyn Read + 'data)],
    out: &mut W,
) -> Result<()> {
    iso::write(out, layout.total_size, |w| build_esp(layout, readers, w))
}

/// Builds a raw GPT disk image from a `Layout` into any `Write` sink.
///
/// # Errors
///
/// Returns an error if ESP construction fails, compression level validation fails, raw image creation fails, or output writing/compression fails.
pub fn build_raw<'data, 'ctx, W: Write>(
    layout: &'ctx Layout<'data>,
    readers: &mut [&'data mut (dyn Read + 'data)],
    out: &mut W,
    compression_level: Option<i32>,
) -> Result<()> {
    if let Some(level) = compression_level {
        let level = validate_compression_level(level)?;
        let mut encoder = zstd::Encoder::new(out, level).map_err(MisoError::ZstdInit)?;
        raw::write(&mut encoder, layout.total_size, |w| {
            build_esp(layout, readers, w)
        })?;
        encoder.finish().map_err(MisoError::Compression)?;
    } else {
        raw::write(out, layout.total_size, |w| build_esp(layout, readers, w))?;
    }

    Ok(())
}

fn build_esp<'data, 'ctx, W: Write>(
    layout: &'ctx Layout<'data>,
    readers: &mut [&'data mut (dyn Read + 'data)],
    writer: &mut W,
) -> Result<()> {
    let mut builder = Builder::new(layout, writer);
    for (file, reader) in layout.files.iter().zip(readers.iter_mut()) {
        builder
            .add_file(file.path, *reader, file.size)
            .map_err(MisoError::Esp)?;
    }

    builder.finish().map_err(MisoError::Esp)
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
