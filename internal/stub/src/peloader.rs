//! Direct PE kernel loader
//!
//! Loads the embedded Linux kernel PE image by manually mapping its sections
//! into memory and jumping to the entry point

use std::ffi::c_void;
use std::ptr;

use anyhow::{Context, Result, bail};
use object::LittleEndian as LE;
use object::pe::{IMAGE_SCN_CNT_CODE, IMAGE_SCN_MEM_EXECUTE};
use uefi::boot::{AllocateType, MemoryType, ScopedProtocol};
use uefi::{Guid, Handle, Status};

use crate::pe::{self, KernelPe};
use crate::{info, warn};

const MEMORY_ATTRIBUTE_GUID: Guid = Guid::parse_or_panic("f4560cf6-40ec-4b4a-a192-bf1d57d0b189");

/// EFI memory attribute bits.
const EFI_MEMORY_RO: u64 = 0x0002_0000;
const EFI_MEMORY_XP: u64 = 0x0000_4000;

type SetMemoryAttributesFn = unsafe extern "efiapi" fn(
    this: *mut MemoryAttributeProtocol,
    base_address: u64,
    length: u64,
    attributes: u64,
) -> Status;

type ClearMemoryAttributesFn = unsafe extern "efiapi" fn(
    this: *mut MemoryAttributeProtocol,
    base_address: u64,
    length: u64,
    attributes: u64,
) -> Status;

type GetMemoryAttributesFn = unsafe extern "efiapi" fn(
    this: *mut MemoryAttributeProtocol,
    base_address: u64,
    length: u64,
    attributes: *mut u64,
) -> Status;

type EfiEntryPoint = unsafe extern "efiapi" fn(Handle, *mut c_void) -> Status;

#[repr(C)]
struct MemoryAttributeProtocol {
    get_memory_attributes: GetMemoryAttributesFn,
    set_memory_attributes: SetMemoryAttributesFn,
    clear_memory_attributes: ClearMemoryAttributesFn,
}

/// Allocates pages and maps PE sections into the allocated buffer
fn map_kernel_sections(kernel: &KernelPe<'_>) -> Result<*mut u8> {
    let page_count = (kernel.size_of_image as usize + 0xFFF) / 0x1000;
    let base_ptr =
        uefi::boot::allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_CODE, page_count)
            .context("failed to allocate pages for kernel image")?
            .as_ptr();

    for section in kernel.sections.iter() {
        let raw_size = section.size_of_raw_data.get(LE) as usize;
        let raw_offset = section.pointer_to_raw_data.get(LE) as usize;
        let virt_addr = section.virtual_address.get(LE) as u64;
        let virt_size = section.virtual_size.get(LE) as usize;

        let dest_offset = virt_addr
            .checked_sub(kernel.image_base)
            .context("section VirtualAddress < ImageBase")?;

        if dest_offset as usize + virt_size > kernel.size_of_image as usize {
            let name = pe::section_name(section);
            bail!(
                "section {name} would write outside allocated memory \
                 (offset=0x{dest_offset:x}, virt_size=0x{virt_size:x}, \
                 image_size=0x{:x})",
                kernel.size_of_image
            );
        }

        let copy_size = raw_size.min(virt_size);

        if copy_size > 0 {
            if raw_offset + copy_size > kernel.data.len() {
                let name = pe::section_name(section);
                bail!(
                    "section {name} raw data out of bounds \
                     (offset=0x{raw_offset:x}, size=0x{copy_size:x}, \
                     data_len=0x{:x})",
                    kernel.data.len()
                );
            }

            // SAFETY: bounds are checked above, base_ptr is freshly allocated
            unsafe {
                ptr::copy_nonoverlapping(
                    kernel.data.as_ptr().add(raw_offset),
                    base_ptr.add(dest_offset as usize),
                    copy_size,
                );
            }
        }

        if virt_size > copy_size {
            // SAFETY: dest_offset + virt_size is bounds-checked above
            unsafe {
                ptr::write_bytes(
                    base_ptr.add(dest_offset as usize + copy_size),
                    0,
                    virt_size - copy_size,
                );
            }
        }
    }

    info!("Mapped kernel at {:p} ({page_count} pages)", base_ptr);
    Ok(base_ptr)
}

/// Locates a protocol by GUID using raw Boot Services FFI.
pub unsafe fn locate_protocol_raw(guid: &Guid) -> Option<*mut c_void> {
    let st = uefi::table::system_table_raw()?;
    // SAFETY: system table and boot services are valid during boot services phase
    let bs = unsafe { &*(*st.as_ptr()).boot_services };

    let mut interface: *mut c_void = ptr::null_mut();
    let status = unsafe {
        (bs.locate_protocol)(
            guid as *const Guid as *const _,
            ptr::null_mut(),
            &mut interface,
        )
    };

    if status == Status::SUCCESS && !interface.is_null() {
        Some(interface)
    } else {
        None
    }
}

/// Sets code sections to RO+X using `EFI_MEMORY_ATTRIBUTE_PROTOCOL`
fn apply_memory_protections(
    proto: *mut MemoryAttributeProtocol,
    base_ptr: *const u8,
    kernel: &KernelPe<'_>,
) {
    for section in kernel.sections.iter() {
        let chars = section.characteristics.get(LE);

        if chars & (IMAGE_SCN_CNT_CODE | IMAGE_SCN_MEM_EXECUTE) == 0 {
            continue;
        }

        let virt_addr = section.virtual_address.get(LE) as u64;
        let virt_size = section.virtual_size.get(LE) as u64;

        let dest_offset = virt_addr.saturating_sub(kernel.image_base);
        let section_base = base_ptr as u64 + dest_offset;

        // Round size up to page boundary for memory attribute operations
        let page_size = (virt_size + 0xFFF) & !0xFFF;

        // SAFETY: proto is valid, section_base points within our allocated pages
        let status = unsafe {
            ((*proto).set_memory_attributes)(proto, section_base, page_size, EFI_MEMORY_RO)
        };
        if status != Status::SUCCESS {
            warn!(
                "Failed to set RO on section {} (status={:?})",
                pe::section_name(section),
                status
            );
            continue;
        }

        // SAFETY: Clear execute protection
        let status = unsafe {
            ((*proto).clear_memory_attributes)(proto, section_base, page_size, EFI_MEMORY_XP)
        };
        if status != Status::SUCCESS {
            warn!(
                "Failed to clear XP on section {} (status={:?})",
                pe::section_name(section),
                status
            );
            // SAFETY: Undo the RO we just set
            unsafe {
                let _ = ((*proto).clear_memory_attributes)(
                    proto,
                    section_base,
                    page_size,
                    EFI_MEMORY_RO,
                );
            }
            continue;
        }

        info!(
            "W^X: section {} at 0x{section_base:x} ({page_size} bytes) -> RO+X",
            pe::section_name(section),
        );
    }
}

/// Converts an ASCII command line to a UCS-2 (UTF-16LE) buffer in pool memory
fn encode_cmdline_ucs2(cmdline: &[u8]) -> Result<(*mut u8, u32)> {
    let cmd = strip_trailing_nuls(cmdline);
    if cmd.is_empty() {
        return Ok((ptr::null_mut(), 0));
    }

    let ucs2_len = cmd.len() + 1; // +1 for null terminator
    let byte_size = ucs2_len * 2;

    let ptr = uefi::boot::allocate_pool(MemoryType::LOADER_DATA, byte_size)
        .context("failed to allocate command line buffer")?
        .as_ptr();

    // SAFETY: ptr is freshly allocated with sufficient size
    unsafe {
        let ucs2 = std::slice::from_raw_parts_mut(ptr as *mut u16, ucs2_len);
        for (i, &byte) in cmd.iter().enumerate() {
            ucs2[i] = byte as u16;
        }
        ucs2[cmd.len()] = 0;
    }

    Ok((ptr, byte_size as u32))
}

fn strip_trailing_nuls(data: &[u8]) -> &[u8] {
    let end = data.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    &data[..end]
}

/// Maps the kernel into memory and transfers control to it
pub fn start(
    kernel: &KernelPe<'_>,
    cmdline: Option<&[u8]>,
    mut loaded_image: ScopedProtocol<uefi::proto::loaded_image::LoadedImage>,
    image_handle: Handle,
) -> Result<()> {
    let loaded_base = map_kernel_sections(kernel)?;

    if kernel.nx_compat {
        // SAFETY: locating a protocol is safe during boot services
        let proto = unsafe { locate_protocol_raw(&MEMORY_ATTRIBUTE_GUID) }
            .map(|p| p as *mut MemoryAttributeProtocol);

        if let Some(proto) = proto {
            info!("EFI_MEMORY_ATTRIBUTE_PROTOCOL available, applying W^X");
            apply_memory_protections(proto, loaded_base, kernel);
        } else {
            warn!("Kernel has NX_COMPAT but EFI_MEMORY_ATTRIBUTE_PROTOCOL not available");
        }
    }

    // SAFETY: loaded_base is valid allocated memory of size_of_image bytes
    unsafe {
        loaded_image.set_image(loaded_base as *const c_void, kernel.size_of_image as u64);
    }

    if let Some(cmdline_bytes) = cmdline {
        let (ptr, size) = encode_cmdline_ucs2(cmdline_bytes)?;
        if !ptr.is_null() {
            // SAFETY: ptr is valid pool memory
            unsafe {
                loaded_image.set_load_options(ptr, size);
            }
            info!("Command line set ({size} bytes UCS-2)");
        }
    }

    let entry_addr = loaded_base as u64 + kernel.entry_point_rva as u64;
    info!("Jumping to kernel entry at 0x{entry_addr:x}");

    drop(loaded_image);

    // SAFETY: entry_addr is within the mapped kernel image. The calling
    // convention matches what a Linux EFI stub kernel expects: the standard
    // EFI_IMAGE_ENTRY_POINT(ImageHandle, SystemTable) signature
    let status = unsafe {
        let entry: EfiEntryPoint = core::mem::transmute(entry_addr as usize);
        entry(
            image_handle,
            std::os::uefi::env::system_table().as_ptr().cast(),
        )
    };

    unreachable!("Kernel entry point returned with status: {:?}", status);
}
