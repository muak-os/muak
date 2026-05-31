use core::ffi::c_void;
use core::ptr;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

use anyhow::{Context as _, Result};
use uefi::boot::{self, MemoryType};
use uefi::{Guid, Handle, Status};

use crate::{error, info};

const LOAD_FILE2_PROTOCOL_GUID: Guid = Guid::parse_or_panic("4006c0c1-fcb3-403e-996d-4a6c8724e06d");

const DEVICE_PATH_PROTOCOL_GUID: Guid =
    Guid::parse_or_panic("09576e91-6d3f-11d2-8e39-00a0c969723b");
const DEVICE_PATH_LEN: usize = 24;

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

static FILE_PTR: AtomicPtr<u8> = AtomicPtr::new(ptr::null_mut());
static FILE_LEN: AtomicUsize = AtomicUsize::new(0);
static LOAD_FILE2_PROTOCOL: LoadFile2Protocol = LoadFile2Protocol {
    load_file: load_file2_callback,
};

/// `LoadFile2` callback implementation.
///
/// This function is called by consumers to load the file data.
/// It follows the UEFI `LoadFile2` protocol semantics:
/// - First call with null buffer returns `BUFFER_TOO_SMALL` and sets `buffer_size`
/// - Second call with adequate buffer copies the data and returns SUCCESS
///
/// # Safety
///
/// This function is called by UEFI firmware with valid pointers. All pointer parameters are
/// guaranteed valid by the UEFI specification.
unsafe extern "efiapi" fn load_file2_callback(
    _this: *mut LoadFile2Protocol,
    _file_path: *const c_void,
    boot_policy: bool,
    buffer_size: *mut usize,
    buffer: *mut u8,
) -> Status {
    info!("[LoadFile2] Callback invoked, boot_policy={boot_policy}");

    if boot_policy {
        error!("[LoadFile2] Rejecting boot_policy=true");
        return Status::UNSUPPORTED;
    }

    let data_ptr = FILE_PTR.load(Ordering::Acquire).cast_const();
    let data_len = FILE_LEN.load(Ordering::Acquire);

    if data_ptr.is_null() || data_len == 0 {
        error!("[LoadFile2] No file data available");
        return Status::NOT_FOUND;
    }

    if buffer_size.is_null() {
        error!("[LoadFile2] buffer_size is null");
        return Status::INVALID_PARAMETER;
    }

    // SAFETY: `buffer_size` was validated as non-null and is provided by UEFI.
    let available_size = unsafe { *buffer_size };
    // SAFETY: `buffer_size` was validated as non-null and is provided by UEFI.
    unsafe {
        *buffer_size = data_len;
    }

    if buffer.is_null() || available_size < data_len {
        info!("[LoadFile2] Returning size: {data_len} bytes");
        return Status::BUFFER_TOO_SMALL;
    }

    info!("[LoadFile2] Copying {data_len} bytes to buffer {buffer:p}");
    // SAFETY: `buffer` is guaranteed valid and sized by the UEFI caller. `data_ptr` and
    // `data_len` are set to valid slice data in `install`.
    unsafe {
        ptr::copy_nonoverlapping(data_ptr, buffer, data_len);
    }

    info!("[LoadFile2] Copy complete, returning SUCCESS");

    Status::SUCCESS
}

/// Builds a vendor media device path with the given GUID.
///
/// The device path consists of:
/// - Vendor media device path node (type 4, subtype 3) with the specified GUID
/// - End of device path node (type 0x7F, subtype 0xFF)
fn build_device_path(guid: &Guid) -> Result<*mut u8> {
    // Device path: 20 bytes (vendor media) + 4 bytes (end node) = 24 bytes
    let dp_ptr = boot::allocate_pool(MemoryType::BOOT_SERVICES_DATA, DEVICE_PATH_LEN)
        .context("Failed to allocate pool for device path")?
        .as_ptr();

    let mut device_path = [0_u8; DEVICE_PATH_LEN];
    device_path
        .get_mut(..4)
        .context("invalid device path header range")?
        .copy_from_slice(&[4, 3, 20, 0]);
    device_path
        .get_mut(4..20)
        .context("invalid device path GUID range")?
        .copy_from_slice(&guid.to_bytes());
    device_path
        .get_mut(20..DEVICE_PATH_LEN)
        .context("invalid device path terminator range")?
        .copy_from_slice(&[0x7F, 0xFF, 4, 0]);

    // SAFETY: `dp_ptr` was allocated with `allocate_pool` and is valid for `DEVICE_PATH_LEN`
    // bytes. The source array has exactly the same length.
    unsafe {
        ptr::copy_nonoverlapping(device_path.as_ptr(), dp_ptr, DEVICE_PATH_LEN);
    }

    Ok(dp_ptr)
}

/// Installs a `LoadFile2` protocol for serving data via a vendor media GUID.
///
/// This creates a new handle with both `DevicePath` and `LoadFile2` protocols installed.
/// Consumers will locate this handle using the specified vendor media GUID
/// and call `LoadFile2` to retrieve the data.
///
/// # Arguments
/// * `data` - The file data to serve
/// * `guid` - The vendor media GUID that identifies this file
///
/// # Errors
///
/// Returns an error if UEFI allocation or protocol installation fails.
pub fn install(data: &[u8], guid: &Guid) -> Result<Handle> {
    info!(
        "Installing LoadFile2 ({} bytes at {:p})",
        data.len(),
        data.as_ptr()
    );

    FILE_PTR.store(data.as_ptr().cast_mut(), Ordering::Release);
    FILE_LEN.store(data.len(), Ordering::Release);

    let dp_ptr = build_device_path(guid)?;

    // SAFETY: Protocol installation follows UEFI specifications and `dp_ptr` points to a valid
    // device path allocation that outlives the boot services phase.
    let handle = unsafe {
        boot::install_protocol_interface(None, &DEVICE_PATH_PROTOCOL_GUID, dp_ptr.cast::<c_void>())
    }
    .context("Failed to install DevicePath protocol")?;

    let protocol_ptr = ptr::from_ref(&LOAD_FILE2_PROTOCOL).cast::<c_void>();
    // SAFETY: `protocol_ptr` points to a static protocol table that remains valid for the boot
    // services phase.
    unsafe {
        boot::install_protocol_interface(Some(handle), &LOAD_FILE2_PROTOCOL_GUID, protocol_ptr)
    }
    .context("Failed to install LoadFile2 protocol")?;

    info!("LoadFile2 installed on handle {:p}", handle.as_ptr());

    Ok(handle)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use uefi::Status;

    use super::*;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn set_file(data: &[u8]) {
        FILE_PTR.store(data.as_ptr().cast_mut(), Ordering::Release);
        FILE_LEN.store(data.len(), Ordering::Release);
    }

    fn clear_file() {
        FILE_PTR.store(ptr::null_mut(), Ordering::Release);
        FILE_LEN.store(0, Ordering::Release);
    }

    fn set_file_with_len(data: &[u8], len: usize) {
        FILE_PTR.store(data.as_ptr().cast_mut(), Ordering::Release);
        FILE_LEN.store(len, Ordering::Release);
    }

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
        clear_file();
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
        clear_file();
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
        set_file_with_len(data, 0);
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
        set_file(data);

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
        set_file(data);
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
        set_file(data);
        let mut buf = [0_u8; 2];
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
        set_file(data);
        let mut buf = vec![0_u8; data.len()];
        let mut sz: usize = buf.len();

        // ACT
        let status = invoke(false, &raw mut sz, buf.as_mut_ptr());

        // ASSERT
        assert_eq!(status, Status::SUCCESS);
        assert_eq!(&buf, data);
    }
}
