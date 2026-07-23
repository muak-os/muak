use std::io::Read;

use koci::arch::Arch;

use crate::build::media;
use crate::build::sources::overlay::OverlayPipes;
use crate::error::Result;

pub(crate) fn iso(
    arch: Arch,
    uki: &mut dyn Read,
    uki_size: u64,
    overlay: &mut OverlayPipes,
    output: &mut (dyn std::io::Write + Send),
) -> Result<()> {
    media::build_iso(
        arch,
        uki,
        uki_size,
        &overlay.files,
        &mut overlay.readers,
        output,
    )
}
