//! Device mapper crypt target support for LUKS2 volumes.

use core::ffi::c_void;
use std::path::Path;

use rustix::fd::OwnedFd;
use rustix::fs::{CWD, FileType, Mode, OFlags, major, makedev, minor, mknodat, open as open_fd};
use rustix::ioctl::{Updater, ioctl};
use zeroize::Zeroize as _;

use super::abi::{
    DM_CONTROL_PATH, DM_DEV_CREATE, DM_DEV_REMOVE, DM_DEV_SUSPEND, DmIoctl, DmTableLoadIoctl,
    ioctl_header_size_u32,
};
use super::table;
use crate::error::{Luks2Error as Error, Result};

pub(crate) struct CryptParams<'a> {
    pub(crate) name: &'a str,
    pub(crate) dm_uuid: &'a str,
    pub(crate) backing_device: &'a str,
    pub(crate) volume_key: &'a [u8],
    pub(crate) cipher: &'a str,
    pub(crate) offset_sectors: u64,
    pub(crate) size_sectors: u64,
    pub(crate) sector_size: u32,
}

/// Creates a new device mapper crypt target with the specified parameters.
pub(crate) fn open(params: &CryptParams<'_>) -> Result<()> {
    let fd = open_fd(DM_CONTROL_PATH, OFlags::RDWR, Mode::empty()).map_err(|error| {
        Error::DeviceMapper(format!("failed to open {DM_CONTROL_PATH}: {error}"))
    })?;

    open_with_fd(&fd, params)
}

/// Closes the device mapper device with the specified name.
pub(crate) fn close(name: &str) -> Result<()> {
    let fd = open_fd(DM_CONTROL_PATH, OFlags::RDWR, Mode::empty()).map_err(|error| {
        Error::DeviceMapper(format!("failed to open {DM_CONTROL_PATH}: {error}"))
    })?;

    close_with_fd(&fd, name)
}

fn open_with_fd(fd: &OwnedFd, params: &CryptParams<'_>) -> Result<()> {
    let mut create_header =
        DmIoctl::with_name_and_uuid(params.name, params.dm_uuid, ioctl_header_size_u32()?)?;
    // SAFETY: `create_header` is a valid `dm_ioctl` payload for `DM_DEV_CREATE`.
    let create_updater = unsafe { Updater::<DM_DEV_CREATE, DmIoctl>::new(&mut create_header) };
    // SAFETY: The updater references a live `DmIoctl` buffer for the duration of the call.
    unsafe { ioctl(fd, create_updater) }
        .map_err(|error| Error::DeviceMapper(format!("DM_DEV_CREATE failed: {error}")))?;

    if let Err(error) = load_table(fd, params) {
        match remove_device(fd, params.name) {
            Ok(()) | Err(_) => {}
        }
        return Err(error);
    }

    let mut resume_header = DmIoctl::with_name(params.name, ioctl_header_size_u32()?)?;
    // SAFETY: `resume_header` is a valid `dm_ioctl` payload for `DM_DEV_SUSPEND` resume.
    let resume_updater = unsafe { Updater::<DM_DEV_SUSPEND, DmIoctl>::new(&mut resume_header) };
    // SAFETY: The updater references a live `DmIoctl` buffer for the duration of the call.
    unsafe { ioctl(fd, resume_updater) }.map_err(|error| {
        match remove_device(fd, params.name) {
            Ok(()) | Err(_) => {}
        }
        Error::DeviceMapper(format!("DM_DEV_SUSPEND (resume) failed: {error}"))
    })?;

    ensure_dev_node(params.name, resume_header.dev)?;

    Ok(())
}

fn close_with_fd(fd: &OwnedFd, name: &str) -> Result<()> {
    remove_device(fd, name)?;

    let path_string = format!("/dev/mapper/{name}");
    match std::fs::remove_file(&path_string) {
        Ok(()) | Err(_) => {}
    }

    Ok(())
}

fn load_table(fd: &OwnedFd, params: &CryptParams<'_>) -> Result<()> {
    let mut buffer = table::build_buffer(params)?;

    let table_load = DmTableLoadIoctl {
        ptr: buffer.as_mut_ptr().cast::<c_void>(),
    };
    // SAFETY: `buffer` contains a complete dm table payload laid out according to the kernel ABI.
    unsafe { ioctl(fd, table_load) }
        .map_err(|error| Error::DeviceMapper(format!("DM_TABLE_LOAD failed: {error}")))?;

    buffer.zeroize();

    Ok(())
}

fn remove_device(fd: &OwnedFd, name: &str) -> Result<()> {
    let mut header = DmIoctl::with_name(name, ioctl_header_size_u32()?)?;
    // SAFETY: `header` is a valid `dm_ioctl` payload for `DM_DEV_REMOVE`.
    let remove_updater = unsafe { Updater::<DM_DEV_REMOVE, DmIoctl>::new(&mut header) };
    // SAFETY: The updater references a live `DmIoctl` buffer for the duration of the call.
    unsafe { ioctl(fd, remove_updater) }
        .map_err(|error| Error::DeviceMapper(format!("DM_DEV_REMOVE failed: {error}")))?;
    Ok(())
}

fn ensure_dev_node(name: &str, dev: u64) -> Result<()> {
    let path_string = format!("/dev/mapper/{name}");
    let path = Path::new(&path_string);

    if path.exists() {
        return Ok(());
    }

    mknodat(
        CWD,
        path,
        FileType::BlockDevice,
        Mode::from_raw_mode(0o660),
        makedev(major(dev), minor(dev)),
    )
    .map_err(|error| Error::DeviceMapper(format!("mknod {path_string} failed: {error}")))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_params(sector_size: u32, volume_key: &[u8]) -> CryptParams<'_> {
        CryptParams {
            name: "crypt-test",
            dm_uuid: "CRYPT-LUKS2-deadbeef",
            backing_device: "/dev/loop0",
            volume_key,
            cipher: "aes-xts-plain64",
            offset_sectors: 1024,
            size_sectors: 2048,
            sector_size,
        }
    }

    #[test]
    fn dm_crypt_close_reports_device_mapper_error_when_control_missing() {
        // ACT
        let result = close("missing-device");

        // ASSERT
        assert!(matches!(result, Err(Error::DeviceMapper(_))));
    }

    #[test]
    fn dm_crypt_open_reports_device_mapper_error_when_control_missing() {
        // ARRANGE
        let volume_key = [0x42_u8; 64];
        let params = test_params(512, &volume_key);

        // ACT
        let result = open(&params);

        // ASSERT
        assert!(matches!(result, Err(Error::DeviceMapper(_))));
    }

    #[test]
    fn dm_crypt_open_with_invalid_fd_reports_error() {
        // ARRANGE
        let fd = open_fd("/dev/null", OFlags::RDWR, Mode::empty()).unwrap();
        let volume_key = [0x42_u8; 64];
        let params = test_params(512, &volume_key);

        // ACT
        let result = open_with_fd(&fd, &params);

        // ASSERT
        assert!(matches!(result, Err(Error::DeviceMapper(_))));
    }

    #[test]
    fn dm_crypt_close_with_invalid_fd_reports_error() {
        // ARRANGE
        let fd = open_fd("/dev/null", OFlags::RDWR, Mode::empty()).unwrap();

        // ACT
        let result = close_with_fd(&fd, "missing-device");

        // ASSERT
        assert!(matches!(result, Err(Error::DeviceMapper(_))));
    }

    #[test]
    fn ensure_dev_node_returns_ok_when_path_exists() {
        // ACT
        let result = ensure_dev_node("", 0);

        // ASSERT
        result.unwrap();
    }
}
