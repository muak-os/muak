//! Device-mapper ioctl interface for dm-crypt setup.
//!
//! Communicates with the kernel's device-mapper subsystem via ioctls on
//! `/dev/mapper/control` to create, configure, and activate dm-crypt mappings.
//! Also provides physical block size detection via `BLKPBSZGET`.

use std::io::{Read, Seek, SeekFrom, Write};

use rustix::fs::{Mode, OFlags, open};
use rustix::ioctl::{Opcode, Updater, ioctl, opcode};
use zeroize::Zeroize;

use crate::constants::{
    BLKPBSZGET, DEFAULT_SECTOR_SIZE, DM_DEV_CREATE_NR, DM_DEV_SUSPEND_NR, DM_IOCTL_TYPE,
    DM_NAME_LEN, DM_UUID_LEN, DM_VERSION,
};
use crate::error::{Error, Result};

const DM_CONTROL_PATH: &str = "/dev/mapper/control";

// dm-ioctl opcodes (read-write, type 0xFD)
const DM_DEV_CREATE: Opcode = opcode::read_write::<DmIoctl>(DM_IOCTL_TYPE, DM_DEV_CREATE_NR);
const DM_DEV_SUSPEND: Opcode = opcode::read_write::<DmIoctl>(DM_IOCTL_TYPE, DM_DEV_SUSPEND_NR);

/// Buffer size for DM_TABLE_LOAD (header + target spec + params).
const DM_TABLE_BUF_SIZE: usize = 16384;

/// dm_ioctl header matching the kernel ABI.
#[repr(C)]
struct DmIoctl {
    version: [u32; 3],
    data_size: u32,
    data_start: u32,
    target_count: u32,
    open_count: i32,
    flags: u32,
    event_nr: u32,
    padding: u32,
    dev: u64,
    name: [u8; DM_NAME_LEN],
    uuid: [u8; DM_UUID_LEN],
    data: [u8; 7],
}

/// dm_target_spec for a single table entry.
#[repr(C)]
struct DmTargetSpec {
    sector_start: u64,
    length: u64,
    status: i32,
    next: u32,
    target_type: [u8; 16],
}

impl DmIoctl {
    fn new(name: &str, uuid: &str, data_size: u32) -> Self {
        let mut hdr = Self {
            version: DM_VERSION,
            data_size,
            data_start: std::mem::size_of::<Self>() as u32,
            target_count: 0,
            open_count: 0,
            flags: 0,
            event_nr: 0,
            padding: 0,
            dev: 0,
            name: [0u8; DM_NAME_LEN],
            uuid: [0u8; DM_UUID_LEN],
            data: [0u8; 7],
        };

        let name_bytes = name.as_bytes();
        let len = name_bytes.len().min(DM_NAME_LEN - 1);
        hdr.name[..len].copy_from_slice(&name_bytes[..len]);

        let uuid_bytes = uuid.as_bytes();
        let len = uuid_bytes.len().min(DM_UUID_LEN - 1);
        hdr.uuid[..len].copy_from_slice(&uuid_bytes[..len]);

        hdr
    }
}

/// Parameters for setting up a dm-crypt mapping.
pub struct CryptParams<'a> {
    pub name: &'a str,
    pub dm_uuid: &'a str,
    pub backing_device: &'a str,
    pub volume_key: &'a [u8],
    pub cipher: &'a str,
    pub offset_sectors: u64,
    pub size_sectors: u64,
    pub sector_size: u32,
}

/// Creates a dm-crypt mapping and activates it.
///
/// This performs three ioctls on `/dev/mapper/control`:
/// 1. `DM_DEV_CREATE` — create the device
/// 2. `DM_TABLE_LOAD` — load the crypt target with cipher, key, and backing device
/// 3. `DM_DEV_SUSPEND` — resume (activate) the device
pub fn dm_crypt_open(params: &CryptParams<'_>) -> Result<()> {
    let fd = open(DM_CONTROL_PATH, OFlags::RDWR, Mode::empty())
        .map_err(|e| Error::DeviceMapper(format!("failed to open {DM_CONTROL_PATH}: {e}")))?;

    // 1. DM_DEV_CREATE
    let mut create_hdr = DmIoctl::new(
        params.name,
        params.dm_uuid,
        std::mem::size_of::<DmIoctl>() as u32,
    );
    // SAFETY: DmIoctl matches the kernel ABI; ioctl is inherently unsafe
    unsafe { ioctl(&fd, Updater::<DM_DEV_CREATE, DmIoctl>::new(&mut create_hdr)) }
        .map_err(|e| Error::DeviceMapper(format!("DM_DEV_CREATE failed: {e}")))?;

    // 2. DM_TABLE_LOAD
    if let Err(e) = dm_table_load(&fd, params) {
        // Clean up on failure: try to remove the device we just created
        let _ = dm_dev_remove(&fd, params.name, params.dm_uuid);
        return Err(e);
    }

    // 3. DM_DEV_SUSPEND (resume)
    let mut resume_hdr = DmIoctl::new(
        params.name,
        params.dm_uuid,
        std::mem::size_of::<DmIoctl>() as u32,
    );
    // flags = 0 means resume (not suspend)
    // SAFETY: DmIoctl matches the kernel ABI
    unsafe {
        ioctl(
            &fd,
            Updater::<DM_DEV_SUSPEND, DmIoctl>::new(&mut resume_hdr),
        )
    }
    .map_err(|e| {
        let _ = dm_dev_remove(&fd, params.name, params.dm_uuid);
        Error::DeviceMapper(format!("DM_DEV_SUSPEND (resume) failed: {e}"))
    })?;

    Ok(())
}

/// Loads the crypt table into a dm device.
///
/// The table format is: `<cipher> <key_hex> <iv_offset> <device> <offset> [<opts>]`
fn dm_table_load(fd: &rustix::fd::OwnedFd, params: &CryptParams<'_>) -> Result<()> {
    // Build the crypt parameters string
    let mut key_hex = hex_encode(params.volume_key);
    let cipher = params.cipher;
    let backing_device = params.backing_device;
    let offset_sectors = params.offset_sectors;
    let sector_size = params.sector_size;
    let params_str = if sector_size != 512 {
        format!(
            "{cipher} {key_hex} 0 {backing_device} {offset_sectors} 3 allow_discards sector_size:{sector_size} no_read_workqueue"
        )
    } else {
        format!(
            "{cipher} {key_hex} 0 {backing_device} {offset_sectors} 2 allow_discards no_read_workqueue"
        )
    };
    key_hex.zeroize();

    let params_bytes = params_str.as_bytes();
    let target_spec_size = std::mem::size_of::<DmTargetSpec>();
    let hdr_size = std::mem::size_of::<DmIoctl>();

    // Total buffer: DmIoctl header + DmTargetSpec + params + null terminator
    let total_size = hdr_size + target_spec_size + params_bytes.len() + 1;
    let buf_size = total_size.max(DM_TABLE_BUF_SIZE);

    let mut buf = vec![0u8; buf_size];

    // Fill in the DmIoctl header at the start
    let hdr = DmIoctl::new(params.name, params.dm_uuid, buf_size as u32);
    // SAFETY: writing repr(C) struct bytes into buffer
    let hdr_bytes =
        unsafe { std::slice::from_raw_parts(&hdr as *const DmIoctl as *const u8, hdr_size) };
    buf[..hdr_size].copy_from_slice(hdr_bytes);

    // Set target_count = 1 in the header
    buf[20..24].copy_from_slice(&1u32.to_ne_bytes());

    // Fill in DmTargetSpec after the header
    let mut target = DmTargetSpec {
        sector_start: 0,
        length: params.size_sectors,
        status: 0,
        next: (target_spec_size + params_bytes.len() + 1) as u32,
        target_type: [0u8; 16],
    };
    let crypt_type = b"crypt";
    target.target_type[..crypt_type.len()].copy_from_slice(crypt_type);

    let target_bytes = unsafe {
        std::slice::from_raw_parts(
            &target as *const DmTargetSpec as *const u8,
            target_spec_size,
        )
    };
    buf[hdr_size..hdr_size + target_spec_size].copy_from_slice(target_bytes);

    // Copy params string after the target spec
    let params_offset = hdr_size + target_spec_size;
    buf[params_offset..params_offset + params_bytes.len()].copy_from_slice(params_bytes);
    // Null terminator
    buf[params_offset + params_bytes.len()] = 0;

    // Issue the ioctl using a raw syscall since the buffer is larger than DmIoctl
    // SAFETY: buf is large enough and laid out per the kernel dm-ioctl ABI
    unsafe {
        ioctl(
            fd,
            Updater::<{ opcode::read_write::<DmIoctl>(DM_IOCTL_TYPE, 9) }, DmIoctl>::new(
                &mut *(buf.as_mut_ptr() as *mut DmIoctl),
            ),
        )
    }
    .map_err(|e| Error::DeviceMapper(format!("DM_TABLE_LOAD failed: {e}")))?;

    // Zeroize buffer (contains key in hex)
    buf.zeroize();

    Ok(())
}

/// Attempts to remove a dm device (best-effort cleanup).
fn dm_dev_remove(fd: &rustix::fd::OwnedFd, name: &str, dm_uuid: &str) -> Result<()> {
    const DM_DEV_REMOVE: Opcode = opcode::read_write::<DmIoctl>(DM_IOCTL_TYPE, 4);

    let mut hdr = DmIoctl::new(name, dm_uuid, std::mem::size_of::<DmIoctl>() as u32);
    // SAFETY: DmIoctl matches the kernel ABI
    unsafe { ioctl(fd, Updater::<DM_DEV_REMOVE, DmIoctl>::new(&mut hdr)) }
        .map_err(|e| Error::DeviceMapper(format!("DM_DEV_REMOVE failed: {e}")))?;
    Ok(())
}

/// Deactivates a dm-crypt mapping by name.
///
/// Clears the device table and removes the device from the device-mapper.
/// The device at `/dev/mapper/<name>` will no longer be accessible after this call.
pub fn dm_crypt_close(name: &str) -> Result<()> {
    let fd = open(DM_CONTROL_PATH, OFlags::RDWR, Mode::empty())
        .map_err(|e| Error::DeviceMapper(format!("failed to open {DM_CONTROL_PATH}: {e}")))?;

    dm_dev_remove(&fd, name, "")
}

/// Detects the physical block size of a device via `BLKPBSZGET` ioctl.
///
/// Falls back to `DEFAULT_SECTOR_SIZE` if detection fails.
pub fn detect_sector_size(device: &str) -> u32 {
    let Ok(fd) = open(device, OFlags::RDONLY, Mode::empty()) else {
        return DEFAULT_SECTOR_SIZE;
    };

    let mut size: u32 = 0;
    // SAFETY: BLKPBSZGET writes a u32 via ioctl
    let result = unsafe {
        ioctl(
            &fd,
            Updater::<{ BLKPBSZGET as Opcode }, u32>::new(&mut size),
        )
    };

    match result {
        Ok(_) if size >= 512 && size.is_power_of_two() => size,
        _ => DEFAULT_SECTOR_SIZE,
    }
}

/// Hex-encodes a byte slice into a lowercase hex string.
fn hex_encode(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for byte in data {
        s.push_str(&format!("{byte:02x}"));
    }
    s
}

/// Reads raw bytes from a device at a specific offset.
pub fn read_device(device: &str, offset: u64, len: usize) -> Result<Vec<u8>> {
    let mut file = std::fs::File::open(device)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; len];
    file.read_exact(&mut buf)?;
    Ok(buf)
}

/// Writes raw bytes to a device at a specific offset.
pub fn write_device(device: &str, offset: u64, data: &[u8]) -> Result<()> {
    let mut file = std::fs::OpenOptions::new().write(true).open(device)?;
    file.seek(SeekFrom::Start(offset))?;
    file.write_all(data)?;
    file.sync_all()?;
    Ok(())
}
