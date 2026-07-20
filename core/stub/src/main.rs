//! UEFI stub for Muak - Loads and starts a kernel from a Unified Kernel Image.

#![feature(uefi_std)]

#[cfg(target_arch = "aarch64")]
mod dtb;
mod loadfile2;
mod log;
mod luks;
mod pe;
mod security;
mod tpm2;

use core::slice;
use std::os::uefi as uefi_std;

use anyhow::{Context as _, Result, anyhow};
use uefi::Guid;
use uefi::Handle;
use uefi::boot::{image_handle, open_protocol_exclusive, set_image_handle};
use uefi::proto::loaded_image::LoadedImage;
use uefi::table::set_system_table;

const LINUX_INITRD_GUID: Guid = Guid::parse_or_panic("5568e427-68fc-4f3d-ac74-ca555231cc68");

/// Initializes the UEFI crate with system table and image handle.
fn setup_uefi_crate() -> Result<()> {
    let st = uefi_std::env::system_table();
    let ih = uefi_std::env::image_handle();

    // SAFETY: UEFI firmware provides a valid system table pointer during boot services phase.
    unsafe {
        set_system_table(st.as_ptr().cast());
    }

    // SAFETY: UEFI firmware provides a valid image handle pointer during boot services phase.
    let ih = unsafe { Handle::from_ptr(ih.as_ptr().cast()) }
        .context("UEFI image handle pointer is null")?;
    // SAFETY: the image handle came from UEFI firmware and is valid during boot services phase.
    unsafe {
        set_image_handle(ih);
    }

    Ok(())
}

/// Entry point for the UEFI stub.
fn main() -> Result<()> {
    setup_uefi_crate()?;

    info!("Muak stub v{} starting...", env!("CARGO_PKG_VERSION"));

    let image_handle = image_handle();

    let loaded_image = open_protocol_exclusive::<LoadedImage>(image_handle)
        .context("Failed to open LoadedImage protocol")?;

    info!(
        "Setup Mode: {}",
        if security::is_setup_mode() {
            "enabled"
        } else {
            "disabled"
        }
    );
    info!(
        "Secure Boot: {}",
        if security::is_secure_boot_enabled() {
            "enabled"
        } else {
            "disabled"
        }
    );

    let (base_addr, image_size) = loaded_image.info();
    info!("Base address: {:p}, size: {}", base_addr, image_size);

    let image_size = usize::try_from(image_size).context("loaded image size exceeds usize")?;
    // SAFETY: base_addr and image_size come from UEFI's LoadedImage protocol, which guarantees the
    // image is valid and loaded in memory for the entire boot services phase. The slice is used
    // only for reading PE section data.
    let image_data = unsafe { slice::from_raw_parts(base_addr.cast::<u8>(), image_size) };
    let sections = pe::uki::Sections::parse(image_data)?;

    for (name, data) in sections.iter_sections() {
        match tpm2::measure_section(name, data) {
            Ok(()) => info!("TPM2: measured {} ({} bytes) into PCR#11", name, data.len()),
            Err(e) => warn!("TPM2: skipping measurement for {}: {}", name, e),
        }
    }

    info!(
        "Kernel: {} bytes at {:p}",
        sections.kernel.len(),
        sections.kernel.as_ptr()
    );

    let kernel = pe::kernel::Image::parse(sections.kernel)?;
    info!(
        "Kernel PE: entry=0x{:x}, base=0x{:x}, size=0x{:x}",
        kernel.entry_point_rva, kernel.base_address, kernel.size
    );

    if let Some(initrd_bytes) = sections.initrd {
        loadfile2::install(initrd_bytes, &LINUX_INITRD_GUID)?;
    }

    #[cfg(target_arch = "aarch64")]
    if let Some(dtb_bytes) = sections.dtb {
        dtb::install(dtb_bytes)?;
    }

    let combined_cmdline: Vec<u8>;
    let cmdline: Option<&[u8]> = if !tpm2::is_available() {
        if let Some(combined) = luks::try_inject(sections.cmdline)? {
            info!("LUKS key read from ESP file");
            combined_cmdline = combined;
            Some(&combined_cmdline)
        } else {
            sections.cmdline
        }
    } else {
        sections.cmdline
    };

    pe::loader::start(&kernel, cmdline, loaded_image, image_handle)?;

    Err(anyhow!(
        "Kernel entry point returned, which should never happen"
    ))
}
