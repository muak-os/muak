//! EFI System Partition (ESP) construction.

use std::io::{Read, Write};

use ::esp::builder::{Builder, Layout};

use crate::error::{MisoError, Result};

/// Builds an EFI System Partition by writing all files from the layout to the writer.
///
/// # Errors
///
/// Returns an error if adding a file to the ESP or finalizing the image fails.
pub fn build<'data, 'ctx, W: Write>(
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
