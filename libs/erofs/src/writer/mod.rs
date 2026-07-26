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

/// Build a complete EROFS image from a planned image plan into a `Write` sink.
///
/// # Errors
///
/// Returns an error when metadata serialization fails or writing data blocks fails.
pub fn image<W: Write>(writer: &mut W, plan: &ImagePlan, config: &MkfsConfig<'_>) -> Result<()> {
    emit::image(writer, plan, config)
}
