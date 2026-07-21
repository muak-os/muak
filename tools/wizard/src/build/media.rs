use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

use esp::FileMeta;
use esp::layout::{Layout, compute};
use koci::arch::Arch;
use miso::{iso, raw};

use crate::arch;
use crate::error::{Result, WizardError};

pub(crate) fn build_iso(
    arch: Arch,
    uki: &mut dyn Read,
    uki_size: u64,
    overlay_files: &[FileMeta<'_>],
    overlay_readers: &mut [UnixStream],
    output: &mut dyn Write,
) -> Result<()> {
    let (layout, mut readers) =
        compute_esp_layout(arch, uki, uki_size, overlay_files, overlay_readers)?;
    iso::build(&layout, &mut readers, &mut DynWriter(output))
        .map_err(|e| WizardError::BuildError(format!("build bootable ISO: {e}")))
}

pub(crate) fn build_raw(
    arch: Arch,
    uki: &mut dyn Read,
    uki_size: u64,
    overlay_files: &[FileMeta<'_>],
    overlay_readers: &mut [UnixStream],
    output: &mut dyn Write,
) -> Result<()> {
    let (layout, mut readers) =
        compute_esp_layout(arch, uki, uki_size, overlay_files, overlay_readers)?;
    raw::build(&layout, &mut readers, &mut DynWriter(output), Some(6))
        .map_err(|e| WizardError::BuildError(format!("build raw disk image: {e}")))
}

fn compute_esp_layout<'a>(
    arch: Arch,
    uki: &'a mut dyn Read,
    uki_size: u64,
    overlay_files: &[FileMeta<'a>],
    overlay_readers: &'a mut [UnixStream],
) -> Result<(Layout<'a>, Vec<&'a mut dyn Read>)> {
    let mut file_metas = Vec::with_capacity(overlay_files.len().saturating_add(1));
    file_metas.push(FileMeta::new(arch::esp(arch).boot_path(), uki_size));
    file_metas.extend_from_slice(overlay_files);

    let mut readers: Vec<&mut dyn Read> = Vec::with_capacity(file_metas.len());
    readers.push(uki);
    for reader in overlay_readers.iter_mut() {
        readers.push(reader);
    }

    let layout = compute(&file_metas)
        .map_err(|e| WizardError::BuildError(format!("compute ESP layout: {e}")))?;

    Ok((layout, readers))
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
