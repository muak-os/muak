use std::ffi::c_void;
use std::ptr;

use anyhow::{Context, Result};
use uefi::boot::{self, MemoryType};
use uefi::{Guid, Handle, Status};

use crate::{error, info};

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
///
/// SAFETY: This function is called by UEFI firmware with valid pointers.
/// All pointer parameters are guaranteed valid by the UEFI specification.
unsafe extern "efiapi" fn load_file2_callback(
    _this: *mut LoadFile2Protocol,
    _file_path: *const c_void,
    boot_policy: bool,
    buffer_size: *mut usize,
    buffer: *mut u8,
) -> Status {
    // SAFETY: This entire function operates on raw pointers passed by UEFI firmware.
    // The UEFI specification guarantees these pointers are valid for the operation.
    // FILE_PTR and FILE_LEN are set by our install() function and remain valid
    // during the boot services phase.
    unsafe {
        info!("[LoadFile2] Callback invoked, boot_policy={}", boot_policy);

        if boot_policy {
            error!("[LoadFile2] Rejecting boot_policy=true");
            return Status::UNSUPPORTED;
        }

        let data_ptr = FILE_PTR;
        let data_len = FILE_LEN;

        if data_ptr.is_null() || data_len == 0 {
            error!("[LoadFile2] No file data available");
            return Status::NOT_FOUND;
        }

        if buffer_size.is_null() {
            error!("[LoadFile2] buffer_size is null");
            return Status::INVALID_PARAMETER;
        }

        let available_size = *buffer_size;
        *buffer_size = data_len;

        if buffer.is_null() || available_size < data_len {
            info!("[LoadFile2] Returning size: {} bytes", data_len);
            return Status::BUFFER_TOO_SMALL;
        }

        info!(
            "[LoadFile2] Copying {} bytes to buffer {:p}",
            data_len, buffer
        );
        // SAFETY: buffer is guaranteed valid and sized by UEFI caller.
        // data_ptr and data_len are set to valid slice data in install().
        std::ptr::copy_nonoverlapping(data_ptr, buffer, data_len);

        info!("[LoadFile2] Copy complete, returning SUCCESS");
        Status::SUCCESS
    }
}

/// Builds a vendor media device path with the given GUID.
///
/// The device path consists of:
/// - Vendor media device path node (type 4, subtype 3) with the specified GUID
/// - End of device path node (type 0x7F, subtype 0xFF)
fn build_device_path(guid: &Guid) -> Result<*mut u8> {
    // Device path: 20 bytes (vendor media) + 4 bytes (end node) = 24 bytes
    let dp_ptr = boot::allocate_pool(MemoryType::BOOT_SERVICES_DATA, 24)
        .context("Failed to allocate pool for device path")?
        .as_ptr();

    // SAFETY: dp_ptr was allocated with allocate_pool and is valid for 24 bytes.
    // All pointer arithmetic stays within bounds. GUID bytes are copied from valid data.
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
pub fn install(data: &[u8], guid: &Guid) -> Result<Handle> {
    info!(
        "Installing LoadFile2 ({} bytes at {:p})",
        data.len(),
        data.as_ptr()
    );

    // SAFETY: All operations are UEFI boot services calls, which are safe during
    // the boot services phase. Memory allocations use valid pool types.
    // Protocol installation follows UEFI specifications.
    unsafe {
        FILE_PTR = data.as_ptr();
        FILE_LEN = data.len();

        let protocol_ptr = boot::allocate_pool(
            MemoryType::BOOT_SERVICES_DATA,
            std::mem::size_of::<LoadFile2Protocol>(),
        )
        .context("Memory allocation failed")?
        .as_ptr() as *mut LoadFile2Protocol;

        (*protocol_ptr).load_file = load_file2_callback;

        let dp_ptr = build_device_path(guid)?;

        let handle = boot::install_protocol_interface(
            None,
            &DEVICE_PATH_PROTOCOL_GUID,
            dp_ptr as *const c_void,
        )
        .context("Failed to install DevicePath protocol")?;

        boot::install_protocol_interface(
            Some(handle),
            &LOAD_FILE2_PROTOCOL_GUID,
            protocol_ptr as *const c_void,
        )
        .context("Failed to install LoadFile2 protocol")?;

        info!("LoadFile2 installed on handle {:p}", handle.as_ptr());

        Ok(handle)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use uefi::Status;

    use super::*;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn invoke(boot_policy: bool, buffer_size: *mut usize, buffer: *mut u8) -> Status {
        // SAFETY: All raw pointer args are controlled by the test; mutable statics
        // are serialised by TEST_LOCK (caller must hold the lock).
        unsafe {
            load_file2_callback(
                ptr::null_mut(),
                ptr::null(),
                boot_policy,
                buffer_size,
                buffer,
            )
        }
    }

    #[test]
    fn boot_policy_true_returns_unsupported() {
        // ARRANGE
        let _g = TEST_LOCK.lock().expect("lock");
        unsafe {
            FILE_PTR = ptr::null();
            FILE_LEN = 0;
        }
        let mut sz: usize = 0;

        // ACT + ASSERT
        assert_eq!(
            invoke(true, &raw mut sz, ptr::null_mut()),
            Status::UNSUPPORTED
        );
    }

    #[test]
    fn null_file_ptr_returns_not_found() {
        // ARRANGE
        let _g = TEST_LOCK.lock().expect("lock");
        unsafe {
            FILE_PTR = ptr::null();
            FILE_LEN = 0;
        }
        let mut sz: usize = 0;

        // ACT + ASSERT
        assert_eq!(
            invoke(false, &raw mut sz, ptr::null_mut()),
            Status::NOT_FOUND
        );
    }

    #[test]
    fn zero_file_len_returns_not_found() {
        // ARRANGE
        let _g = TEST_LOCK.lock().expect("lock");
        let data = b"x";
        unsafe {
            FILE_PTR = data.as_ptr();
            FILE_LEN = 0;
        }
        let mut sz: usize = 0;

        // ACT + ASSERT
        assert_eq!(
            invoke(false, &raw mut sz, ptr::null_mut()),
            Status::NOT_FOUND
        );
    }

    #[test]
    fn null_buffer_size_returns_invalid_parameter() {
        // ARRANGE
        let _g = TEST_LOCK.lock().expect("lock");
        let data = b"hello";
        unsafe {
            FILE_PTR = data.as_ptr();
            FILE_LEN = data.len();
        }

        // ACT + ASSERT
        assert_eq!(
            invoke(false, ptr::null_mut(), ptr::null_mut()),
            Status::INVALID_PARAMETER
        );
    }

    #[test]
    fn null_buffer_returns_buffer_too_small_and_sets_size() {
        // ARRANGE
        let _g = TEST_LOCK.lock().expect("lock");
        let data = b"hello";
        unsafe {
            FILE_PTR = data.as_ptr();
            FILE_LEN = data.len();
        }
        let mut sz: usize = 0;

        // ACT
        let status = invoke(false, &raw mut sz, ptr::null_mut());

        // ASSERT
        assert_eq!(status, Status::BUFFER_TOO_SMALL);
        assert_eq!(sz, data.len());
    }

    #[test]
    fn undersized_buffer_returns_buffer_too_small() {
        // ARRANGE
        let _g = TEST_LOCK.lock().expect("lock");
        let data = b"hello";
        unsafe {
            FILE_PTR = data.as_ptr();
            FILE_LEN = data.len();
        }
        let mut buf = [0u8; 2];
        let mut sz: usize = buf.len();

        // ACT
        let status = invoke(false, &raw mut sz, buf.as_mut_ptr());

        // ASSERT
        assert_eq!(status, Status::BUFFER_TOO_SMALL);
        assert_eq!(sz, data.len());
    }

    #[test]
    fn sufficient_buffer_copies_data_and_returns_success() {
        // ARRANGE
        let _g = TEST_LOCK.lock().expect("lock");
        let data = b"hello world";
        unsafe {
            FILE_PTR = data.as_ptr();
            FILE_LEN = data.len();
        }
        let mut buf = vec![0u8; data.len()];
        let mut sz: usize = buf.len();

        // ACT
        let status = invoke(false, &raw mut sz, buf.as_mut_ptr());

        // ASSERT
        assert_eq!(status, Status::SUCCESS);
        assert_eq!(&buf, data);
    }
}
