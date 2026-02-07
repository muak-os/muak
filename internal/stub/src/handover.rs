//! Linux EFI Handover Protocol implementation for x86_64

use std::ffi::c_void;
use std::os::uefi as uefi_std;

use anyhow::{Context, Result, bail};
use uefi::Handle;
use uefi::boot::{self, AllocateType, MemoryType};

use crate::info;

// Linux boot protocol magic values
const BOOT_FLAG: u16 = 0xAA55;
const HDRS_MAGIC: u32 = 0x5372_6448;
const MIN_BOOT_PROTOCOL: u16 = 0x020B; // 2.11

// xloadflags bits
const XLF_EFI_HANDOVER_64: u16 = 1 << 3;

// boot_params extension fields
const OFF_EXT_RAMDISK_IMAGE: usize = 0x0C0;
const OFF_EXT_RAMDISK_SIZE: usize = 0x0C4;
const OFF_EXT_CMD_LINE_PTR: usize = 0x0C8;

const BOOT_PARAMS_SIZE: usize = 4096;

// Setup header field offsets
const OFF_SETUP_SECTS: usize = 0x1F1;
const OFF_BOOT_FLAG: usize = 0x1FE;
const OFF_HEADER: usize = 0x202;
const OFF_VERSION: usize = 0x206;
const OFF_TYPE_OF_LOADER: usize = 0x210;
const OFF_LOADFLAGS: usize = 0x211;
const OFF_CODE32_START: usize = 0x214;
const OFF_RAMDISK_IMAGE: usize = 0x218;
const OFF_RAMDISK_SIZE: usize = 0x21C;
const OFF_CMD_LINE_PTR: usize = 0x228;
const OFF_RELOCATABLE: usize = 0x234;
const OFF_XLOADFLAGS: usize = 0x236;
const OFF_HANDOVER_OFFSET: usize = 0x264;

const SETUP_HEADER_OFFSET: usize = 0x1F1;
const SETUP_HEADER_COPY_END: usize = 0x268;

fn read_u8(data: &[u8], offset: usize) -> u8 {
    data[offset]
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn write_u8(data: &mut [u8], offset: usize, val: u8) {
    data[offset] = val;
}

fn write_u32(data: &mut [u8], offset: usize, val: u32) {
    let bytes = val.to_le_bytes();
    data[offset..offset + 4].copy_from_slice(&bytes);
}

/// Validates that the kernel supports the 64-bit EFI handover protocol
pub fn validate(kernel: &[u8]) -> Result<()> {
    if kernel.len() < SETUP_HEADER_COPY_END + 4 {
        bail!("kernel image too small for Linux boot protocol");
    }

    let boot_flag = read_u16(kernel, OFF_BOOT_FLAG);
    if boot_flag != BOOT_FLAG {
        bail!("invalid boot_flag: expected 0x{BOOT_FLAG:04X}, got 0x{boot_flag:04X}");
    }

    let header = read_u32(kernel, OFF_HEADER);
    if header != HDRS_MAGIC {
        bail!("invalid header magic: expected 0x{HDRS_MAGIC:08X}, got 0x{header:08X}");
    }

    let version = read_u16(kernel, OFF_VERSION);
    if version < MIN_BOOT_PROTOCOL {
        bail!("boot protocol version 0x{version:04X} too old (need >= 0x{MIN_BOOT_PROTOCOL:04X})");
    }

    let relocatable = read_u8(kernel, OFF_RELOCATABLE);
    if relocatable == 0 {
        bail!("kernel is not relocatable");
    }

    if version >= 0x020C {
        let xloadflags = read_u16(kernel, OFF_XLOADFLAGS);
        if xloadflags & XLF_EFI_HANDOVER_64 == 0 {
            bail!("kernel does not support 64-bit EFI handover (xloadflags=0x{xloadflags:04X})");
        }
    }

    info!(
        "Handover supported: protocol={}.{}, handover_offset=0x{:X}",
        version >> 8,
        version & 0xFF,
        read_u32(kernel, OFF_HANDOVER_OFFSET)
    );

    Ok(())
}

/// Allocates and populates the `boot_params` (zero page) structure
pub fn setup_boot_params(
    kernel: &[u8],
    cmdline: Option<&[u8]>,
    initrd: Option<&[u8]>,
) -> Result<*mut u8> {
    let params_ptr = boot::allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, 1)
        .context("failed to allocate boot_params page")?
        .as_ptr();

    let params =
        // SAFETY: params_ptr was just allocated as one full page (4096 bytes) by UEFI.
        unsafe { std::slice::from_raw_parts_mut(params_ptr, BOOT_PARAMS_SIZE) };

    params.fill(0);

    let copy_len = SETUP_HEADER_COPY_END - SETUP_HEADER_OFFSET;
    params[SETUP_HEADER_OFFSET..SETUP_HEADER_OFFSET + copy_len]
        .copy_from_slice(&kernel[SETUP_HEADER_OFFSET..SETUP_HEADER_COPY_END]);

    write_u8(params, OFF_TYPE_OF_LOADER, 0xFF);

    let loadflags = read_u8(params, OFF_LOADFLAGS);
    write_u8(params, OFF_LOADFLAGS, loadflags | 0x01);

    let setup_sects = {
        let s = read_u8(kernel, OFF_SETUP_SECTS);
        if s == 0 { 4u32 } else { s as u32 }
    };
    let code32_start = kernel.as_ptr() as u64 + (setup_sects + 1) as u64 * 512;
    write_u32(params, OFF_CODE32_START, code32_start as u32);

    if let Some(cmdline_bytes) = cmdline {
        let cmd = strip_trailing_nuls(cmdline_bytes);
        if !cmd.is_empty() {
            let cmd_ptr = boot::allocate_pool(MemoryType::LOADER_DATA, cmd.len() + 1)
                .context("failed to allocate command line")?
                .as_ptr();
            // SAFETY: cmd_ptr is freshly allocated with sufficient size
            unsafe {
                std::ptr::copy_nonoverlapping(cmd.as_ptr(), cmd_ptr, cmd.len());
                // NUL-terminate
                *cmd_ptr.add(cmd.len()) = 0;
            }

            let addr = cmd_ptr as u64;
            write_u32(params, OFF_CMD_LINE_PTR, addr as u32);
            write_u32(params, OFF_EXT_CMD_LINE_PTR, (addr >> 32) as u32);

            info!("Cmdline at 0x{addr:X} ({} bytes)", cmd.len());
        }
    }

    if let Some(initrd_bytes) = initrd {
        if !initrd_bytes.is_empty() {
            let addr = initrd_bytes.as_ptr() as u64;
            let size = initrd_bytes.len() as u64;

            write_u32(params, OFF_RAMDISK_IMAGE, addr as u32);
            write_u32(params, OFF_EXT_RAMDISK_IMAGE, (addr >> 32) as u32);
            write_u32(params, OFF_RAMDISK_SIZE, size as u32);
            write_u32(params, OFF_EXT_RAMDISK_SIZE, (size >> 32) as u32);

            info!("Initrd at 0x{addr:X} ({size} bytes)");
        }
    }

    Ok(params_ptr)
}

/// Computes the handover entry point and jumps into the kernel
pub fn execute(image_handle: Handle, kernel: &[u8], boot_params: *mut u8) -> ! {
    let setup_sects = {
        let s = read_u8(kernel, OFF_SETUP_SECTS);
        if s == 0 { 4u32 } else { s as u32 }
    };
    let handover_offset = read_u32(kernel, OFF_HANDOVER_OFFSET);

    // Entry = kernel_base + (setup_sects+1)*512 + 512 + handover_offset
    // The +512 is for the 64-bit entry (x86_64 adds 0x200 to the 32-bit entry)
    let kernel_base = kernel.as_ptr() as u64;
    let entry_addr = kernel_base + (setup_sects + 1) as u64 * 512 + 512 + handover_offset as u64;

    info!(
        "Jumping to handover entry at 0x{entry_addr:X} (base=0x{kernel_base:X}, \
         setup_sects={setup_sects}, handover_offset=0x{handover_offset:X})"
    );

    let system_table = uefi_std::env::system_table().as_ptr();

    type HandoverEntry =
        unsafe extern "sysv64" fn(handle: *mut c_void, sys_table: *mut c_void, params: *mut u8);

    // SAFETY: The entry address was computed from the validated kernel image.
    // The kernel's EFI handover entry expects exactly these three arguments
    // in System V calling convention. This call never returns.
    unsafe {
        let entry: HandoverEntry = core::mem::transmute(entry_addr as usize);
        entry(image_handle.as_ptr(), system_table.cast(), boot_params);
    }

    unreachable!("handover entry returned");
}

fn strip_trailing_nuls(data: &[u8]) -> &[u8] {
    let end = data.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    &data[..end]
}
