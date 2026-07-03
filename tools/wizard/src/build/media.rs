//! Bootable media builders.

use std::io::{Cursor, Read, Write};
use std::sync::Arc;

use esp::model::{Arch as EspArch, EspFile, EspSpec};

use super::source::OverlayEntry;
use crate::error::{Result, WizardError};

/// Build an ISO image from scratch, writing directly to a `Write` sink.
///
/// # Errors
///
/// Returns an error when creating the ISO or writing it fails.
pub fn write_iso<W: Write>(
    arch: EspArch,
    uki_reader: &mut impl Read,
    uki_size: u64,
    writer: &mut W,
) -> Result<()> {
    let boot = EspFile::boot(arch, uki_reader, uki_size);
    let mut spec = EspSpec::builder()
        .add_file(boot)
        .map_err(|e| WizardError::BuildError(format!("add UKI to ISO ESP spec: {e}")))?
        .build()
        .map_err(|e| WizardError::BuildError(format!("build ISO ESP spec: {e}")))?;
    miso::build_iso(&mut spec, writer)
        .map_err(|e| WizardError::BuildError(format!("build bootable ISO: {e}")))?;

    Ok(())
}

/// Build a raw disk image from scratch, writing directly to a `Write` sink.
///
/// # Errors
///
/// Returns an error when creating the raw image or writing it fails.
pub fn write_raw<W: Write>(
    arch: EspArch,
    uki_reader: &mut impl Read,
    uki_size: u64,
    overlay_entries: Vec<OverlayEntry>,
    writer: &mut W,
) -> Result<()> {
    let count = overlay_entries.len();
    let mut cursors: Vec<Cursor<Arc<[u8]>>> = Vec::with_capacity(count);
    for entry in &overlay_entries {
        cursors.push(Cursor::new(entry.data.clone()));
    }

    let mut overlay_files: Vec<EspFile<'_>> = Vec::with_capacity(count);
    for (file, entry) in cursors.iter_mut().zip(overlay_entries) {
        overlay_files.push(EspFile {
            path: entry.path,
            reader: file,
            size: entry.size,
        });
    }

    let mut builder = EspSpec::builder();
    let boot = EspFile::boot(arch, uki_reader, uki_size);
    builder = builder
        .add_file(boot)
        .map_err(|e| WizardError::BuildError(format!("add UKI to raw ESP spec: {e}")))?;

    for file in overlay_files {
        builder = builder
            .add_file(file)
            .map_err(|e| WizardError::BuildError(format!("add overlay file: {e}")))?;
    }

    let mut spec = builder
        .build()
        .map_err(|e| WizardError::BuildError(format!("build raw ESP spec: {e}")))?;

    miso::build_raw(&mut spec, writer, Some(6))
        .map_err(|e| WizardError::BuildError(format!("build raw disk image: {e}")))?;

    Ok(())
}
