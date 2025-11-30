#![no_std]
#![no_main]

mod error;
mod loader;
mod loadfile2;
mod log;
mod pe;

use uefi::Guid;
use uefi::prelude::*;
use uefi::proto::loaded_image::LoadedImage;

use crate::error::{StubError, StubResult};
use crate::pe::UkiSections;

const LINUX_INITRD_GUID: Guid = Guid::parse_or_panic("5568e427-68fc-4f3d-ac74-ca555231cc68");

#[entry]
fn main() -> Status {
    uefi::helpers::init().unwrap();
    log_info!("Muak stub v0.2.0 starting...");

    match run() {
        Ok(status) => status,
        Err(e) => {
            log_error!("Stub error: {}", e);
            e.to_status()
        }
    }
}

fn run() -> StubResult<Status> {
    let image_handle = uefi::boot::image_handle();

    // Get base address of loaded image
    let loaded_image = uefi::boot::open_protocol_exclusive::<LoadedImage>(image_handle)
        .map_err(|_| StubError::ProtocolOpenFailed)?;

    let (base_addr, _image_size) = loaded_image.info();
    log_info!("Base address: {:p}", base_addr);

    // Parse UKI sections
    let sections = unsafe { UkiSections::parse(base_addr as *const u8)? };
    let kernel_bytes = sections.require_kernel()?;

    log_info!(
        "Kernel: {} bytes at {:p}",
        kernel_bytes.len(),
        kernel_bytes.as_ptr()
    );

    // Install LoadFile2 protocol for initrd if present
    if let Some(initrd_bytes) = sections.initrd {
        loadfile2::install(initrd_bytes, &LINUX_INITRD_GUID)?;
    }

    // Load and configure kernel
    let kernel_handle = loader::load_kernel(image_handle, kernel_bytes)?;

    if let Some(cmdline_bytes) = sections.cmdline {
        loader::set_cmdline(kernel_handle, cmdline_bytes)?;
    }

    Ok(loader::start(kernel_handle))
}
