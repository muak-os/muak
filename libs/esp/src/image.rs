//! ESP image construction from a precomputed layout.

use std::io::{Read, Write};

use fatfs::builder::{build as fat_build, precompute};

use crate::error::{EspError, Result};
use crate::layout::Layout;

/// Builds a complete FAT ESP image from a [`Layout`] by streaming file data from `readers`.
///
/// Readers must be provided in the same order as [`Layout::files`].
///
/// # Errors
///
/// Returns an error when:
/// - The number of readers doesn't match the number of files in the layout
/// - Writing the FAT filesystem fails
pub fn build<'data, W: Write>(
    layout: &Layout<'data>,
    readers: &mut [&'data mut (dyn Read + 'data)],
    writer: &mut W,
) -> Result<()> {
    if readers.len() != layout.files.len() {
        return Err(EspError::Incomplete {
            expected: layout.files.len(),
            actual: readers.len(),
        });
    }

    let precomputed = precompute(&layout.files, layout.total_size)?;
    fat_build(&precomputed, readers, writer)?;

    Ok(())
}
