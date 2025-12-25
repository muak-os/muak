use core::ffi::c_void;
use core::ptr;

use uefi::boot::{self, MemoryType};
use uefi::table::system_table_raw;
use uefi::{Guid, Handle, Status};

use crate::error::{StubError, StubResult};
use crate::{log_error, log_info, log_warn};

#[inline]
unsafe fn uefi_copy_mem(dest: *mut u8, src: *const u8, len: usize) {
    let st = system_table_raw().expect("system table not available");
    let st = st.as_ptr();
    // SAFETY: System table and boot services are valid during boot services phase
    unsafe {
        let bt = (*st).boot_services;
        ((*bt).copy_mem)(dest, src, len);
    }
}

const LOAD_FILE2_PROTOCOL_GUID: Guid = Guid::parse_or_panic("4006c0c1-fcb3-403e-996d-4a6c8724e06d");

const DEVICE_PATH_PROTOCOL_GUID: Guid =
    Guid::parse_or_panic("09576e91-6d3f-11d2-8e39-00a0c969723b");

#[repr(C)]
struct LoadFile2Protocol {
    load_file: unsafe extern "efiapi" fn(
        this: *mut LoadFile2Protocol,
        file_path: *const c_void,
        boot_policy: bool,
        buffer_size: *mut usize,
        buffer: *mut u8,
    ) -> Status,
}

static mut FILE_PTR: *const u8 = ptr::null();
static mut FILE_LEN: usize = 0;

/// LoadFile2 callback implementation.
///
/// This function is called by consumers to load the file data.
/// It follows the UEFI LoadFile2 protocol semantics:
/// - First call with null buffer returns BUFFER_TOO_SMALL and sets buffer_size
/// - Second call with adequate buffer copies the data and returns SUCCESS
unsafe extern "efiapi" fn load_file2_callback(
    _this: *mut LoadFile2Protocol,
    _file_path: *const c_void,
    boot_policy: bool,
    buffer_size: *mut usize,
    buffer: *mut u8,
) -> Status {
    // SAFETY: This entire function operates on raw pointers passed by UEFI firmware.
    // The caller guarantees these pointers are valid.
    unsafe {
        log_info!("[LoadFile2] Callback invoked, boot_policy={}", boot_policy);

        if boot_policy {
            log_warn!("[LoadFile2] Rejecting boot_policy=true");
            return Status::UNSUPPORTED;
        }

        let data_ptr = FILE_PTR;
        let data_len = FILE_LEN;

        if data_ptr.is_null() || data_len == 0 {
            log_error!("[LoadFile2] No file data available");
            return Status::NOT_FOUND;
        }

        if buffer_size.is_null() {
            log_error!("[LoadFile2] buffer_size is null");
            return Status::INVALID_PARAMETER;
        }

        let available_size = *buffer_size;
        *buffer_size = data_len;

        // First call: caller queries the size
        if buffer.is_null() || available_size < data_len {
            log_info!("[LoadFile2] Returning size: {} bytes", data_len);
            return Status::BUFFER_TOO_SMALL;
        }

        log_info!(
            "[LoadFile2] Copying {} bytes to buffer {:p}",
            data_len,
            buffer
        );
        uefi_copy_mem(buffer, data_ptr, data_len);

        log_info!("[LoadFile2] Copy complete, returning SUCCESS");
        Status::SUCCESS
    }
}

/// Builds a vendor media device path with the given GUID.
///
/// The device path consists of:
/// - Vendor media device path node (type 4, subtype 3) with the specified GUID
/// - End of device path node (type 0x7F, subtype 0xFF)
fn build_device_path(guid: &Guid) -> StubResult<*mut u8> {
    // Device path: 20 bytes (vendor media) + 4 bytes (end node) = 24 bytes
    let dp_ptr = boot::allocate_pool(MemoryType::BOOT_SERVICES_DATA, 24)
        .map_err(|_| StubError::AllocationFailed)?
        .as_ptr();

    unsafe {
        let dp = dp_ptr;

        // Vendor media device path node
        // Type 4 (Media), SubType 3 (Vendor), Length 20
        *dp.add(0) = 4; // Type: Media Device Path
        *dp.add(1) = 3; // SubType: Vendor
        *dp.add(2) = 20; // Length low byte
        *dp.add(3) = 0; // Length high byte

        // Copy GUID bytes
        let guid_bytes = guid.to_bytes();
        ptr::copy_nonoverlapping(guid_bytes.as_ptr(), dp.add(4), 16);

        // End of device path node
        // Type 0x7F (End), SubType 0xFF (End Entire), Length 4
        *dp.add(20) = 0x7F;
        *dp.add(21) = 0xFF;
        *dp.add(22) = 4;
        *dp.add(23) = 0;
    }

    Ok(dp_ptr)
}

/// Installs a LoadFile2 protocol for serving data via a vendor media GUID.
///
/// This creates a new handle with both DevicePath and LoadFile2 protocols installed.
/// Consumers will locate this handle using the specified vendor media GUID
/// and call LoadFile2 to retrieve the data.
///
/// # Arguments
/// * `data` - The file data to serve
/// * `guid` - The vendor media GUID that identifies this file
pub fn install(data: &[u8], guid: &Guid) -> StubResult<Handle> {
    log_info!(
        "Installing LoadFile2 ({} bytes at {:p})",
        data.len(),
        data.as_ptr()
    );

    unsafe {
        FILE_PTR = data.as_ptr();
        FILE_LEN = data.len();

        let protocol_ptr = boot::allocate_pool(
            MemoryType::BOOT_SERVICES_DATA,
            core::mem::size_of::<LoadFile2Protocol>(),
        )
        .map_err(|_| StubError::AllocationFailed)?
        .as_ptr() as *mut LoadFile2Protocol;

        (*protocol_ptr).load_file = load_file2_callback;

        let dp_ptr = build_device_path(guid)?;

        let handle = boot::install_protocol_interface(
            None,
            &DEVICE_PATH_PROTOCOL_GUID,
            dp_ptr as *const c_void,
        )
        .map_err(|_| StubError::ProtocolInstallFailed)?;

        boot::install_protocol_interface(
            Some(handle),
            &LOAD_FILE2_PROTOCOL_GUID,
            protocol_ptr as *const c_void,
        )
        .map_err(|_| StubError::ProtocolInstallFailed)?;

        log_info!("LoadFile2 installed on handle {:p}", handle.as_ptr());

        Ok(handle)
    }
}
