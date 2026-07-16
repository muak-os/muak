//! Bootable media builder.

use std::io::{Read, Write};

use esp::FileMeta;
use esp::builder::compute_layout;
use koci::arch::Arch;

use crate::arch;
use crate::error::{Result, WizardError};

/// Writes a UKI and optional overlay files into bootable media.
///
/// # Errors
///
/// Returns an error when writing the media fails.
pub(crate) fn write(
    arch: Arch,
    uki: &mut dyn Read,
    uki_size: u64,
    overlay_files: &[FileMeta<'_>],
    overlay_readers: Option<&mut [&mut dyn Read]>,
    iso: Option<&mut dyn Write>,
    raw: Option<&mut dyn Write>,
) -> Result<()> {
    let mut file_metas = Vec::with_capacity(overlay_files.len().saturating_add(1));
    file_metas.push(FileMeta::new(arch::esp(arch).boot_path(), uki_size));
    file_metas.extend_from_slice(overlay_files);

    let mut readers: Vec<&mut dyn Read> = Vec::with_capacity(file_metas.len());
    readers.push(uki);
    if let Some(overlay_readers) = overlay_readers {
        for reader in overlay_readers {
            readers.push(*reader);
        }
    }

    let layout = compute_layout(&file_metas)
        .map_err(|e| WizardError::BuildError(format!("compute ESP layout: {e}")))?;

    if let Some(writer) = iso {
        miso::build_iso(&layout, &mut readers, &mut DynWriter(writer))
            .map_err(|e| WizardError::BuildError(format!("build bootable ISO: {e}")))
    } else if let Some(writer) = raw {
        miso::build_raw(&layout, &mut readers, &mut DynWriter(writer), Some(6))
            .map_err(|e| WizardError::BuildError(format!("build raw disk image: {e}")))
    } else {
        Ok(())
    }
}

struct DynWriter<'a>(&'a mut dyn Write);

impl Write for DynWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}
