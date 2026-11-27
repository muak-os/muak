//! EROFS image serialization split by data kind and helper role.

mod compressed;
mod data;
mod dir;
mod emit;
mod inode;
mod sizes;

use std::io::Write;

use crate::MkfsConfig;
use crate::error::Result;
use crate::layout::ImagePlan;
use crate::source::SizedFile;

/// Emit a complete EROFS image from a layout-only plan and positional file readers.
///
/// # Errors
///
/// Returns an error when metadata serialization fails, a reader ends early, or a
/// re-compressed pcluster length drifts from the recorded layout.
pub fn image<W: Write>(
    writer: &mut W,
    plan: &ImagePlan,
    files: &mut [SizedFile<'_>],
    config: &MkfsConfig<'_>,
) -> Result<()> {
    emit::image(writer, plan, files, config)
}
