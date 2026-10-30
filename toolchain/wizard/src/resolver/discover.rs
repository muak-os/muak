//! Discovery of registry-backed details for a resolution.

use disk::layout::Layout;

use crate::domain::overlay;
use crate::domain::resolution::Resolution;
use crate::error::Result;
use crate::nodes::{disk_layout_annotation, measured_sizes};

/// Discovers every overlay asset from the overlay image's eStargz table of
/// contents and stores them on the resolution in image entry order.
///
/// # Errors
///
/// Returns an error when the overlay image cannot be measured or an entry
/// references a malformed placement.
pub(crate) fn assets(resolution: &mut Resolution) -> Result<()> {
    let Some(overlay) = resolution.build().overlay() else {
        return Ok(());
    };
    let entries = measured_sizes(&overlay.source, overlay.arch)?;
    let assets = overlay::classify(overlay, entries)?;
    resolution.set_overlay_assets(Some(assets));

    Ok(())
}

/// Resolves the disk layout of a resolution onto it.
///
/// # Errors
///
/// Returns an error when the annotations cannot be fetched or name an unknown
/// layout.
pub fn layout(resolution: &mut Resolution) -> Result<()> {
    let layout = match resolution.build().overlay() {
        Some(overlay) => match disk_layout_annotation(&overlay.source, overlay.arch)? {
            Some(name) => Layout::by_name(&name)?,
            None => Layout::Uefi,
        },
        None => Layout::Uefi,
    };

    resolution.set_layout(layout);

    Ok(())
}
