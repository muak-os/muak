#![feature(uefi_std)]

#[cfg(target_arch = "aarch64")]
mod dtb;
mod loadfile2;
mod log;
mod pe;
mod peloader;
mod security;

use anyhow::{Context, Result};
use std::os::uefi as uefi_std;
use uefi::Guid;
use uefi::Handle;
use uefi::proto::loaded_image::LoadedImage;

use crate::pe::{KernelPe, UkiSections};

const LINUX_INITRD_GUID: Guid = Guid::parse_or_panic("5568e427-68fc-4f3d-ac74-ca555231cc68");

fn setup_uefi_crate() {
    let st = uefi_std::env::system_table();
    let ih = uefi_std::env::image_handle();

    // SAFETY: UEFI firmware provides valid system table and image handle pointers
    // during the boot services phase. This is required setup for the `uefi` crate.
    unsafe {
        uefi::table::set_system_table(st.as_ptr().cast());

        let ih = Handle::from_ptr(ih.as_ptr().cast()).expect("Something's very wrong");
        uefi::boot::set_image_handle(ih);
    }
}

fn main() -> Result<()> {
    setup_uefi_crate();

    info!("Muak stub v{} starting...", env!("CARGO_PKG_VERSION"));

    let image_handle = uefi::boot::image_handle();

    let loaded_image = uefi::boot::open_protocol_exclusive::<LoadedImage>(image_handle)
        .context("Failed to open LoadedImage protocol")?;

    info!("Setup Mode: {}", security::is_setup_mode());
    info!("Secure Boot: {}", security::is_secure_boot_enabled());

    let (base_addr, image_size) = loaded_image.info();
    info!("Base address: {:p}, size: {}", base_addr, image_size);

    let image_data =
        // SAFETY: base_addr and image_size come from UEFI's LoadedImage protocol,
        // which guarantees the image is valid and loaded in memory for the entire
        // boot services phase. The slice is used only for reading PE section data.
        unsafe { std::slice::from_raw_parts(base_addr as *const u8, image_size as usize) };
    let sections = UkiSections::parse(image_data)?;
    let kernel_bytes = sections.require_kernel()?;

    info!(
        "Kernel: {} bytes at {:p}",
        kernel_bytes.len(),
        kernel_bytes.as_ptr()
    );

    let kernel = KernelPe::parse(kernel_bytes)?;
    info!(
        "Kernel PE: entry=0x{:x}, base=0x{:x}, size=0x{:x}",
        kernel.entry_point_rva, kernel.image_base, kernel.size_of_image
    );

    if let Some(initrd_bytes) = sections.initrd {
        loadfile2::install(initrd_bytes, &LINUX_INITRD_GUID)?;
    }

    #[cfg(target_arch = "aarch64")]
    if let Some(dtb_bytes) = sections.dtb {
        dtb::install(dtb_bytes)?;
    }

    peloader::start(&kernel, sections.cmdline, loaded_image, image_handle)?;

    unreachable!("Kernel entry point returned, which should never happen");
}
