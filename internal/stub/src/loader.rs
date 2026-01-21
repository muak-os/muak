use anyhow::{Result, anyhow};
use uefi::boot::{self, LoadImageSource, MemoryType};
use uefi::proto::loaded_image::LoadedImage;
use uefi::{Handle, Status};

use crate::{log_error, log_info, log_warn};

pub fn load_kernel(image_handle: Handle, kernel_bytes: &[u8]) -> Result<Handle> {
    log_info!("Loading kernel image...");

    let kernel_handle = boot::load_image(
        image_handle,
        LoadImageSource::FromBuffer {
            buffer: kernel_bytes,
            file_path: None,
        },
    )
    .map_err(|_| anyhow!("failed to load kernel image"))?;

    log_info!("Kernel loaded, handle: {:p}", kernel_handle.as_ptr());
    Ok(kernel_handle)
}

pub fn set_cmdline(kernel_handle: Handle, cmdline: &[u8]) -> Result<()> {
    let cmd_str = core::str::from_utf8(cmdline)
        .unwrap_or("")
        .trim_matches(char::from(0));

    if cmd_str.is_empty() {
        return Ok(());
    }

    log_info!("Setting cmdline: {}", cmd_str);

    // Convert to UCS-2
    let char_count = cmd_str.chars().count() + 1; // +1 for null terminator
    let byte_size = char_count * 2;

    let cmdline_ptr = boot::allocate_pool(MemoryType::LOADER_DATA, byte_size)
        .map_err(|_| anyhow!("memory allocation failed"))?
        .as_ptr() as *mut u16;

    // Copy characters as UCS-2
    unsafe {
        for (i, ch) in cmd_str.chars().enumerate() {
            *cmdline_ptr.add(i) = ch as u16;
        }
        *cmdline_ptr.add(char_count - 1) = 0; // Null terminator
    }

    let mut loaded_image = boot::open_protocol_exclusive::<LoadedImage>(kernel_handle)
        .map_err(|_| anyhow!("failed to open protocol"))?;

    unsafe {
        loaded_image.set_load_options(cmdline_ptr as *const u8, byte_size as u32);
    }

    log_info!("Cmdline set ({} bytes)", byte_size);
    Ok(())
}

pub fn start(kernel_handle: Handle) -> Status {
    log_info!("Starting kernel...");
    let result = boot::start_image(kernel_handle);

    // We should never get here - kernel doesn't return
    match result {
        Ok(_) => {
            log_warn!("Kernel returned unexpectedly with success");
            Status::SUCCESS
        }
        Err(e) => {
            log_error!("Kernel returned with error: {:?}", e);
            e.status()
        }
    }
}
