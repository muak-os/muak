use anyhow::{Context, Result};
use std::ffi::OsStr;
use std::os::uefi::ffi::OsStrExt;
use uefi::Handle;
use uefi::boot::{self, LoadImageSource, MemoryType};
use uefi::proto::loaded_image::LoadedImage;

use crate::info;

pub fn load_kernel(image_handle: Handle, kernel_bytes: &[u8]) -> Result<Handle> {
    info!("Loading kernel image...");

    let kernel_handle = boot::load_image(
        image_handle,
        LoadImageSource::FromBuffer {
            buffer: kernel_bytes,
            file_path: None,
        },
    )
    .context("Failed to load kernel image")?;

    info!("Kernel loaded, handle: {:p}", kernel_handle.as_ptr());
    Ok(kernel_handle)
}

pub fn set_cmdline(kernel_handle: Handle, cmdline: &[u8]) -> Result<()> {
    let cmd_str = std::str::from_utf8(cmdline)
        .unwrap_or("")
        .trim_matches(char::from(0));

    if cmd_str.is_empty() {
        return Ok(());
    }

    info!("Setting cmdline: {}", cmd_str);

    let wide_chars: Vec<u16> = OsStr::new(cmd_str).encode_wide().collect();

    let byte_size = wide_chars.len() * 2;
    let cmdline_ptr = boot::allocate_pool(MemoryType::LOADER_DATA, byte_size)
        .context("Failed to allocate pool for cmdline")?
        .as_ptr() as *mut u16;

    unsafe {
        std::ptr::copy_nonoverlapping(wide_chars.as_ptr(), cmdline_ptr, wide_chars.len());
    }

    let mut loaded_image = boot::open_protocol_exclusive::<LoadedImage>(kernel_handle)
        .context("failed to open LoadedImage protocol")?;

    unsafe {
        loaded_image.set_load_options(cmdline_ptr as *const u8, byte_size as u32);
    }

    info!("Cmdline set ({} bytes)", byte_size);
    Ok(())
}

pub fn start(kernel_handle: Handle) -> Result<()> {
    info!("Starting kernel...");
    boot::start_image(kernel_handle).context("Failed to start kernel image")
}
