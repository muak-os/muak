use std::io::Read;

use koci::arch::Arch;

use crate::build::media;
use crate::build::sources::overlay::OverlayPipes;
use crate::error::Result;

/// Writes a UKI binary and overlay files into a raw disk image.
pub(crate) fn raw(
    arch: Arch,
    uki: &mut dyn Read,
    uki_size: u64,
    overlay: &mut OverlayPipes,
    output: &mut dyn std::io::Write,
) -> Result<()> {
    media::build_raw(
        arch,
        uki,
        uki_size,
        &overlay.files,
        &mut overlay.readers,
        output,
    )
}
