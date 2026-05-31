//! Apply memory protections to loaded kernel sections using `EFI_MEMORY_ATTRIBUTE_PROTOCOL`.

use object::LittleEndian as LE;
use object::pe::{IMAGE_SCN_CNT_CODE, IMAGE_SCN_MEM_EXECUTE, ImageSectionHeader};
use uefi::{Guid, Status};

use crate::pe::kernel::{Image, section_name};
use crate::pe::loader::protocol;
use crate::warn;

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

#[repr(C)]
pub(super) struct MemoryAttributeProtocol {
    get: GetMemoryAttributesFn,
    set: SetMemoryAttributesFn,
    clear: ClearMemoryAttributesFn,
}

pub(super) fn locate() -> Option<*mut MemoryAttributeProtocol> {
    // SAFETY: locating a protocol is safe during boot services.
    unsafe { protocol::locate_raw(&MEMORY_ATTRIBUTE_GUID) }
        .map(<*mut core::ffi::c_void>::cast::<MemoryAttributeProtocol>)
}

/// Sets code sections to RO+X using `EFI_MEMORY_ATTRIBUTE_PROTOCOL`.
pub(super) fn apply(proto: *mut MemoryAttributeProtocol, base_ptr: *const u8, kernel: &Image<'_>) {
    for section in kernel.sections.iter() {
        let chars = section.characteristics.get(LE);

        if chars & (IMAGE_SCN_CNT_CODE | IMAGE_SCN_MEM_EXECUTE) == 0 {
            continue;
        }

        let virt_addr = u64::from(section.virtual_address.get(LE));
        let virt_size = u64::from(section.virtual_size.get(LE));

        let dest_offset = virt_addr.saturating_sub(kernel.base_address);
        let Some(section_base) =
            usize_to_u64(base_ptr.addr()).and_then(|base_addr| base_addr.checked_add(dest_offset))
        else {
            warn!(
                "Skipping W^X for section {}: address overflow",
                section_name(section)
            );
            continue;
        };

        let Some(page_size) = virt_size.checked_add(0xFFF).map(|size| size & !0xFFF) else {
            warn!(
                "Skipping W^X for section {}: size overflow",
                section_name(section)
            );
            continue;
        };

        // SAFETY: proto is a valid firmware-provided `EFI_MEMORY_ATTRIBUTE_PROTOCOL` pointer.
        let set_memory_attributes = unsafe { (*proto).set };
        // SAFETY: proto is valid, section_base points within our allocated pages.
        let status =
            unsafe { set_memory_attributes(proto, section_base, page_size, EFI_MEMORY_RO) };
        if status != Status::SUCCESS {
            warn!(
                "Failed to set RO on section {} (status={:?})",
                section_name(section),
                status
            );
            continue;
        }

        // SAFETY: proto is a valid firmware-provided `EFI_MEMORY_ATTRIBUTE_PROTOCOL` pointer.
        let clear_memory_attributes = unsafe { (*proto).clear };
        // SAFETY: proto is valid, section_base points within our allocated pages.
        let status =
            unsafe { clear_memory_attributes(proto, section_base, page_size, EFI_MEMORY_XP) };
        if status != Status::SUCCESS {
            warn!(
                "Failed to clear XP on section {} (status={:?})",
                section_name(section),
                status
            );
            // SAFETY: proto is valid and this best-effort call undoes the RO attribute set above.
            let rollback_status: Status =
                unsafe { clear_memory_attributes(proto, section_base, page_size, EFI_MEMORY_RO) };
            warn_rollback_failure(section, rollback_status);
            continue;
        }

        crate::info!(
            "W^X: section {} at 0x{section_base:x} ({page_size} bytes) -> RO+X",
            section_name(section),
        );
    }
}

fn warn_rollback_failure(section: &ImageSectionHeader, status: Status) {
    if status == Status::SUCCESS {
        return;
    }

    warn!(
        "Failed to roll back RO on section {} (status={:?})",
        section_name(section),
        status
    );
}

fn usize_to_u64(value: usize) -> Option<u64> {
    u64::try_from(value).ok()
}
