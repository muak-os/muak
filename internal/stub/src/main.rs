#![feature(uefi_std)]

mod loader;
mod loadfile2;
mod log;
mod pe;

use anyhow::{Result, anyhow};
use std::os::uefi as uefi_std;
use uefi::Guid;
use uefi::Handle;
use uefi::proto::loaded_image::LoadedImage;

use crate::pe::UkiSections;

const LINUX_INITRD_GUID: Guid = Guid::parse_or_panic("5568e427-68fc-4f3d-ac74-ca555231cc68");

/// Performs the necessary setup code for the `uefi` crate.
fn setup_uefi_crate() {
    let st = uefi_std::env::system_table();
    let ih = uefi_std::env::image_handle();

    // Mandatory setup code for `uefi` crate.
    unsafe {
        uefi::table::set_system_table(st.as_ptr().cast());

        let ih = Handle::from_ptr(ih.as_ptr().cast()).expect("Something's very wrong");
        uefi::boot::set_image_handle(ih);
    }
}

fn main() -> Result<()> {
    setup_uefi_crate();
    log_info!("Muak stub v{} starting...", env!("CARGO_PKG_VERSION"));

    let image_handle = uefi::boot::image_handle();

    let loaded_image = uefi::boot::open_protocol_exclusive::<LoadedImage>(image_handle)
        .map_err(|_| anyhow!("failed to open protocol"))?;

    let (base_addr, _image_size) = loaded_image.info();
    log_info!("Base address: {:p}", base_addr);

    let sections = unsafe { UkiSections::parse(base_addr as *const u8)? };
    let kernel_bytes = sections.require_kernel()?;

    log_info!(
        "Kernel: {} bytes at {:p}",
        kernel_bytes.len(),
        kernel_bytes.as_ptr()
    );

    if let Some(initrd_bytes) = sections.initrd {
        loadfile2::install(initrd_bytes, &LINUX_INITRD_GUID)?;
    }

    let kernel_handle = loader::load_kernel(image_handle, kernel_bytes)?;

    if let Some(cmdline_bytes) = sections.cmdline {
        loader::set_cmdline(kernel_handle, cmdline_bytes)?;
    }

    let _ = loader::start(kernel_handle);

    Ok(())
}
