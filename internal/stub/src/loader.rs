use anyhow::{Context, Result};
use std::ffi::OsStr;
use std::os::uefi::ffi::OsStrExt;
use uefi::Handle;
use uefi::boot::{self, LoadImageSource, MemoryType};
use uefi::proto::device_path::DevicePath;
use uefi::proto::loaded_image::LoadedImage;

use crate::info;

/// Memory-mapped device path node structure.
/// Layout follows UEFI specification for Hardware Device Path (Type 1), Memory Mapped (SubType 3).
#[repr(C, packed)]
struct MemoryMappedDevicePath {
    // Device path node header
    dp_type: u8,     // 0x01 = Hardware Device Path
    dp_subtype: u8,  // 0x03 = Memory Mapped
    length: [u8; 2], // Length of this node (24 bytes)
    // Memory mapped specific fields
    memory_type: u32, // EFI memory type
    start_address: u64,
    end_address: u64,
}

/// End of device path node.
#[repr(C, packed)]
struct EndDevicePath {
    dp_type: u8,    // 0x7F = End of Hardware Device Path
    dp_subtype: u8, // 0xFF = End Entire Device Path
    length: [u8; 2],
}

/// Combined device path structure for kernel loading.
#[repr(C, packed)]
struct KernelDevicePath {
    memory_mapped: MemoryMappedDevicePath,
    end: EndDevicePath,
}

/// Builds a memory-mapped device path for the kernel buffer.
fn build_kernel_device_path(kernel_bytes: &[u8]) -> KernelDevicePath {
    let start_addr = kernel_bytes.as_ptr() as u64;
    let end_addr = start_addr + kernel_bytes.len() as u64;

    KernelDevicePath {
        memory_mapped: MemoryMappedDevicePath {
            dp_type: 0x01,    // Hardware Device Path
            dp_subtype: 0x03, // Memory Mapped
            length: [24, 0],  // 24 bytes for this node
            memory_type: 1,   // EfiLoaderData = 1
            start_address: start_addr,
            end_address: end_addr,
        },
        end: EndDevicePath {
            dp_type: 0x7F,    // End of Hardware Device Path
            dp_subtype: 0xFF, // End Entire Device Path
            length: [4, 0],   // 4 bytes for end node
        },
    }
}

pub fn load_kernel(image_handle: Handle, kernel_bytes: &[u8]) -> Result<Handle> {
    info!("Loading kernel image...");

    let device_path_data = build_kernel_device_path(kernel_bytes);

    let dp_bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(
            &device_path_data as *const KernelDevicePath as *const u8,
            std::mem::size_of::<KernelDevicePath>(),
        )
    };

    // SAFETY: The device path bytes are correctly formatted according to UEFI spec
    let device_path = unsafe { DevicePath::from_ffi_ptr(dp_bytes.as_ptr().cast()) };

    info!(
        "Kernel device path: start=0x{:x}, end=0x{:x}",
        kernel_bytes.as_ptr() as u64,
        kernel_bytes.as_ptr() as u64 + kernel_bytes.len() as u64
    );

    let kernel_handle = boot::load_image(
        image_handle,
        LoadImageSource::FromBuffer {
            buffer: kernel_bytes,
            file_path: Some(device_path),
        },
    )
    .context("Failed to load kernel image")?;

    info!("Kernel loaded, handle: {:p}", kernel_handle.as_ptr());
    Ok(kernel_handle)
}

pub fn set_cmdline(kernel_handle: Handle, cmdline: &[u8]) -> Result<()> {
    let cmd_str = std::str::from_utf8(cmdline)
        .unwrap_or("console=tty0 console=ttyS0 init=/init")
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

    // SAFETY: cmdline_ptr was allocated with allocate_pool and is valid for the
    // requested byte_size. The source pointer is from a Vec, guaranteed valid.
    unsafe {
        std::ptr::copy_nonoverlapping(wide_chars.as_ptr(), cmdline_ptr, wide_chars.len());
    }

    let mut loaded_image = boot::open_protocol_exclusive::<LoadedImage>(kernel_handle)
        .context("failed to open LoadedImage protocol")?;

    // SAFETY: cmdline_ptr points to valid allocated memory from allocate_pool.
    // The UEFI LoadedImage protocol accepts raw pointers for load options.
    unsafe {
        loaded_image.set_load_options(cmdline_ptr as *const u8, byte_size as u32);
    }

    Ok(())
}

pub fn start(kernel_handle: Handle) -> Result<()> {
    info!("Starting kernel...");
    boot::start_image(kernel_handle).context("Failed to start kernel image")
}
