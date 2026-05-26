//! ABI definitions and utilities for interacting with the Linux device mapper.

use core::ffi::c_void;
use core::mem::size_of;
use core::ptr;
use core::slice;

use rustix::io::Result as RustixResult;
use rustix::ioctl::{Ioctl, IoctlOutput, Opcode, opcode};

use crate::error::{Luks2Error as Error, Result};

pub(super) const BLKPBSZGET: u32 = 0x127B;
pub(super) const DEFAULT_SECTOR_SIZE: u32 = 4096;
const DM_DEV_CREATE_NR: u8 = 3;
pub(super) const DM_DEV_REMOVE_NR: u8 = 4;
const DM_DEV_SUSPEND_NR: u8 = 6;
pub(super) const DM_TABLE_LOAD_NR: u8 = 9;
const DM_IOCTL_TYPE: u8 = 0xFD;
pub(super) const DM_NAME_LEN: usize = 128;
pub(super) const DM_UUID_LEN: usize = 129;
const DM_VERSION: [u32; 3] = [4, 0, 0];

pub(super) const DM_CONTROL_PATH: &str = "/dev/mapper/control";
pub(super) const DM_TABLE_BUF_SIZE: usize = 16_384;
pub(super) const TARGET_TYPE: &[u8] = b"crypt";

pub(super) const DM_DEV_CREATE: Opcode =
    opcode::read_write::<DmIoctl>(DM_IOCTL_TYPE, DM_DEV_CREATE_NR);
pub(super) const DM_DEV_SUSPEND: Opcode =
    opcode::read_write::<DmIoctl>(DM_IOCTL_TYPE, DM_DEV_SUSPEND_NR);
pub(super) const DM_DEV_REMOVE: Opcode =
    opcode::read_write::<DmIoctl>(DM_IOCTL_TYPE, DM_DEV_REMOVE_NR);
pub(super) const DM_TABLE_LOAD: Opcode =
    opcode::read_write::<DmIoctl>(DM_IOCTL_TYPE, DM_TABLE_LOAD_NR);
pub(super) const BLKPBSZGET_OPCODE: Opcode = BLKPBSZGET;

pub(super) struct DmTableLoadIoctl {
    pub(super) ptr: *mut c_void,
}

// SAFETY: This wrapper passes a live mutable dm table buffer to the kernel using the expected ABI.
unsafe impl Ioctl for DmTableLoadIoctl {
    type Output = ();
    const IS_MUTATING: bool = true;

    fn opcode(&self) -> Opcode {
        DM_TABLE_LOAD
    }

    fn as_ptr(&mut self) -> *mut c_void {
        self.ptr
    }

    unsafe fn output_from_ptr(_out: IoctlOutput, _ptr: *mut c_void) -> RustixResult<Self::Output> {
        Ok(())
    }
}

#[repr(C)]
pub(super) struct DmIoctl {
    version: [u32; 3],
    data_size: u32,
    data_start: u32,
    pub(super) target_count: u32,
    open_count: i32,
    flags: u32,
    event_nr: u32,
    padding: u32,
    pub(super) dev: u64,
    pub(super) name: [u8; DM_NAME_LEN],
    pub(super) uuid: [u8; DM_UUID_LEN],
    data: [u8; 7],
}

#[repr(C)]
pub(super) struct DmTargetSpec {
    pub(super) sector_start: u64,
    pub(super) length: u64,
    pub(super) status: i32,
    pub(super) next: u32,
    pub(super) target_type: [u8; 16],
}

impl DmIoctl {
    pub(super) fn with_name(name: &str, data_size: u32) -> Result<Self> {
        let mut header = Self {
            version: DM_VERSION,
            data_size,
            data_start: ioctl_header_size_u32()?,
            target_count: 0,
            open_count: 0,
            flags: 0,
            event_nr: 0,
            padding: 0,
            dev: 0,
            name: [0_u8; DM_NAME_LEN],
            uuid: [0_u8; DM_UUID_LEN],
            data: [0_u8; 7],
        };

        copy_c_string(&mut header.name, name.as_bytes());

        Ok(header)
    }

    pub(super) fn with_name_and_uuid(name: &str, uuid: &str, data_size: u32) -> Result<Self> {
        let mut header = Self::with_name(name, data_size)?;
        copy_c_string(&mut header.uuid, uuid.as_bytes());
        Ok(header)
    }
}

pub(super) fn ioctl_header_size_u32() -> Result<u32> {
    usize_to_u32(size_of::<DmIoctl>())
}

pub(super) fn usize_to_u32(value: usize) -> Result<u32> {
    u32::try_from(value).map_err(|_error| Error::InvalidField("value exceeds u32".into()))
}

pub(super) fn copy_c_string(dst: &mut [u8], src: &[u8]) {
    copy_prefix(
        dst.get_mut(..dst.len().saturating_sub(1))
            .unwrap_or(&mut []),
        src,
    );
}

pub(super) fn copy_prefix(dst: &mut [u8], src: &[u8]) {
    let prefix_len = dst.len().min(src.len());
    if let (Some(dst_prefix), Some(src_prefix)) = (dst.get_mut(..prefix_len), src.get(..prefix_len))
    {
        dst_prefix.copy_from_slice(src_prefix);
    }
}

pub(super) fn dm_ioctl_bytes(header: &DmIoctl) -> &[u8] {
    let ptr = ptr::from_ref(header).cast::<u8>();
    // SAFETY: `DmIoctl` is `#[repr(C)]` and this exposes exactly its initialized bytes.
    unsafe { slice::from_raw_parts(ptr, size_of::<DmIoctl>()) }
}

pub(super) fn dm_target_spec_bytes(target: &DmTargetSpec) -> &[u8] {
    let ptr = ptr::from_ref(target).cast::<u8>();
    // SAFETY: `DmTargetSpec` is `#[repr(C)]` and this exposes exactly its initialized bytes.
    unsafe { slice::from_raw_parts(ptr, size_of::<DmTargetSpec>()) }
}

#[cfg(test)]
mod tests {
    use core::ffi::c_void;
    use core::mem::size_of;
    use core::ptr;

    use super::*;

    #[test]
    fn ioctl_with_name_truncates_name_and_null_terminates() {
        // ARRANGE
        let long_name = "n".repeat(DM_NAME_LEN + 8);

        // ACT
        let header = DmIoctl::with_name(&long_name, 4096).unwrap();

        // ASSERT
        assert_eq!(header.name[DM_NAME_LEN - 1], 0);
        assert_eq!(header.name[..DM_NAME_LEN - 1], [b'n'; DM_NAME_LEN - 1]);
    }

    #[test]
    fn ioctl_with_name_and_uuid_truncates_uuid_and_null_terminates() {
        // ARRANGE
        let long_uuid = "u".repeat(DM_UUID_LEN + 8);

        // ACT
        let header = DmIoctl::with_name_and_uuid("name", &long_uuid, 4096).unwrap();

        // ASSERT
        assert_eq!(header.uuid[DM_UUID_LEN - 1], 0);
        assert_eq!(header.uuid[..DM_UUID_LEN - 1], [b'u'; DM_UUID_LEN - 1]);
    }

    #[test]
    fn ioctl_wrapper_exposes_expected_opcode_and_pointer() {
        // ARRANGE
        let mut byte = 0_u8;
        let mut ioctl_wrapper = DmTableLoadIoctl {
            ptr: ptr::from_mut(&mut byte).cast::<c_void>(),
        };

        // ACT
        let opcode = ioctl_wrapper.opcode();
        let raw_ptr = ioctl_wrapper.as_ptr();

        // ASSERT
        assert_eq!(opcode, DM_TABLE_LOAD);
        assert_eq!(raw_ptr, ptr::from_mut(&mut byte).cast::<c_void>());
        assert!(DmTableLoadIoctl::IS_MUTATING);
    }

    #[test]
    fn usize_to_u32_accepts_small_value() {
        // ACT
        let result = usize_to_u32(1234);

        // ASSERT
        assert_eq!(result.unwrap(), 1234);
    }

    #[test]
    fn copy_prefix_uses_destination_length() {
        // ARRANGE
        let mut dst = [0_u8; 4];

        // ACT
        copy_prefix(&mut dst, b"abcdef");

        // ASSERT
        assert_eq!(dst, *b"abcd");
    }

    #[test]
    fn copy_c_string_preserves_null_terminator() {
        // ARRANGE
        let mut dst = [0_u8; 5];

        // ACT
        copy_c_string(&mut dst, b"abcdef");

        // ASSERT
        assert_eq!(&dst[..4], b"abcd");
        assert_eq!(dst[4], 0);
    }

    #[test]
    fn dm_ioctl_bytes_matches_struct_size() {
        // ARRANGE
        let header = DmIoctl::with_name("name", 4096).unwrap();

        // ACT
        let bytes = dm_ioctl_bytes(&header);

        // ASSERT
        assert_eq!(bytes.len(), size_of::<DmIoctl>());
    }

    #[test]
    fn dm_target_spec_bytes_matches_struct_size() {
        // ARRANGE
        let target = DmTargetSpec {
            sector_start: 0,
            length: 1,
            status: 0,
            next: 2,
            target_type: [0_u8; 16],
        };

        // ACT
        let bytes = dm_target_spec_bytes(&target);

        // ASSERT
        assert_eq!(bytes.len(), size_of::<DmTargetSpec>());
    }
}
