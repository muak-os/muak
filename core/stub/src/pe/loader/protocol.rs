//! UEFI protocol handling.

use core::ffi::c_void;
use core::ptr;

use uefi::table::system_table_raw;
use uefi::{Guid, Status};

/// Locates a protocol by GUID using raw Boot Services FFI.
///
/// # Safety
///
/// The caller must only use this during the boot services phase and must cast the returned pointer
/// to the protocol matching `guid`.
#[must_use]
pub(crate) unsafe fn locate_raw(guid: &Guid) -> Option<*mut c_void> {
    let st = system_table_raw()?;
    let st_ptr = st.as_ptr();
    // SAFETY: system table is valid during boot services phase.
    let boot_services = unsafe { (*st_ptr).boot_services };
    // SAFETY: boot services are valid during boot services phase.
    let boot_services = unsafe { &*boot_services };

    let mut interface: *mut c_void = ptr::null_mut();
    let guid_ptr = ptr::from_ref(guid).cast::<Guid>();
    // SAFETY: boot services are valid, `guid_ptr` points to a GUID, and `interface` is a valid
    // output pointer.
    let status =
        unsafe { (boot_services.locate_protocol)(guid_ptr, ptr::null_mut(), &raw mut interface) };

    (status == Status::SUCCESS && !interface.is_null()).then_some(interface)
}
