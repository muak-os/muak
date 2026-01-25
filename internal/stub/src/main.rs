#![feature(uefi_std)]

mod loader;
mod loadfile2;
mod log;
mod pe;

use anyhow::{Context, Result};
use std::os::uefi as uefi_std;
use uefi::CStr16;
use uefi::Guid;
use uefi::Handle;
use uefi::proto::loaded_image::LoadedImage;
use uefi::runtime::VariableVendor;

use crate::pe::UkiSections;

const LINUX_INITRD_GUID: Guid = Guid::parse_or_panic("5568e427-68fc-4f3d-ac74-ca555231cc68");

fn setup_uefi_crate() {
    let st = uefi_std::env::system_table();
    let ih = uefi_std::env::image_handle();

    // SAFETY: UEFI firmware provides valid system table and image handle pointers
    // during the boot services phase. This is required setup for the uefi crate.
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

    let mut name_buf = [0u16; 10];
    let name = CStr16::from_str_with_buf("SetupMode", &mut name_buf).expect("Invalid SetupMode");
    let mut buf = [0u8; 1];
    let setup_mode =
        match uefi::runtime::get_variable(&name, &VariableVendor::GLOBAL_VARIABLE, &mut buf) {
            Ok((data, _)) => data[0],
            Err(_) => 0,
        };
    info!("SetupMode: {}", setup_mode);

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

    if let Some(initrd_bytes) = sections.initrd {
        loadfile2::install(initrd_bytes, &LINUX_INITRD_GUID)?;
    }

    let kernel_handle = loader::load_kernel(image_handle, kernel_bytes)?;

    if let Some(cmdline_bytes) = sections.cmdline {
        loader::set_cmdline(kernel_handle, cmdline_bytes)?;
    }

    loader::start(kernel_handle)?;

    unreachable!("If we're here, something went wrong");
}
