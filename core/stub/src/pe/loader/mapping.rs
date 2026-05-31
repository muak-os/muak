//! Functions for mapping PE sections into memory.

use core::ptr;

use anyhow::{Context as _, Result, bail};
use object::LittleEndian as LE;
use object::pe::ImageSectionHeader;
use uefi::boot::{AllocateType, MemoryType, allocate_pages};

use crate::info;
use crate::pe::kernel::{Image, section_name};

/// Allocates pages and maps PE sections into the allocated buffer.
pub(super) fn map_sections(kernel: &Image<'_>) -> Result<*mut u8> {
    let image_size = usize::try_from(kernel.size).context("kernel image size exceeds usize")?;
    let page_count = image_size.div_ceil(0x1000);
    let base_ptr = allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_CODE, page_count)
        .context("failed to allocate pages for kernel image")?
        .as_ptr();

    for section in kernel.sections.iter() {
        let raw_size =
            usize::try_from(section.size_of_raw_data.get(LE)).context("raw size exceeds usize")?;
        let raw_offset = usize::try_from(section.pointer_to_raw_data.get(LE))
            .context("raw offset exceeds usize")?;
        let virt_addr = u64::from(section.virtual_address.get(LE));
        let virt_size =
            usize::try_from(section.virtual_size.get(LE)).context("virtual size exceeds usize")?;

        let dest_offset = virt_addr
            .checked_sub(kernel.base_address)
            .context("section VirtualAddress < ImageBase")?;
        let dest_offset = usize::try_from(dest_offset).context("section offset exceeds usize")?;
        let dest_end = dest_offset
            .checked_add(virt_size)
            .context("section virtual bounds overflow")?;

        if dest_end > image_size {
            let name = section_name(section);
            bail!(
                "section {name} would write outside allocated memory \
                 (offset=0x{dest_offset:x}, virt_size=0x{virt_size:x}, image_size=0x{:x})",
                kernel.size
            );
        }

        let copy_size = raw_size.min(virt_size);
        copy_section_data(
            kernel,
            section,
            base_ptr,
            raw_offset,
            dest_offset,
            copy_size,
        )?;
        zero_section_tail(base_ptr, dest_offset, virt_size, copy_size)?;
    }

    info!("Mapped kernel at {:p} ({page_count} pages)", base_ptr);

    Ok(base_ptr)
}

fn copy_section_data(
    kernel: &Image<'_>,
    section: &ImageSectionHeader,
    base_ptr: *mut u8,
    raw_offset: usize,
    dest_offset: usize,
    copy_size: usize,
) -> Result<()> {
    if copy_size == 0 {
        return Ok(());
    }

    let raw_end = raw_offset
        .checked_add(copy_size)
        .context("section raw bounds overflow")?;
    if raw_end > kernel.data.len() {
        let name = section_name(section);
        bail!(
            "section {name} raw data out of bounds \
             (offset=0x{raw_offset:x}, size=0x{copy_size:x}, data_len=0x{:x})",
            kernel.data.len()
        );
    }

    let source = kernel
        .data
        .get(raw_offset..raw_end)
        .context("section raw data unavailable")?;
    let destination = base_ptr.wrapping_add(dest_offset);

    // SAFETY: bounds are checked above, base_ptr is freshly allocated, and source and destination
    // do not overlap.
    unsafe {
        ptr::copy_nonoverlapping(source.as_ptr(), destination, copy_size);
    }

    Ok(())
}

fn zero_section_tail(
    base_ptr: *mut u8,
    dest_offset: usize,
    virt_size: usize,
    copy_size: usize,
) -> Result<()> {
    if virt_size <= copy_size {
        return Ok(());
    }

    let zero_offset = dest_offset
        .checked_add(copy_size)
        .context("section zero-fill offset overflow")?;
    let zero_len = virt_size
        .checked_sub(copy_size)
        .context("section zero-fill length underflow")?;
    let destination = base_ptr.wrapping_add(zero_offset);

    // SAFETY: dest_offset + virt_size is bounds-checked by the caller.
    unsafe {
        ptr::write_bytes(destination, 0, zero_len);
    }

    Ok(())
}
