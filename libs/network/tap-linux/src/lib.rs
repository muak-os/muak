//! TUN/TAP device adapter that owns the queue descriptor and leaves bridge policy to the caller.

#![warn(missing_docs)]

pub mod error;

use std::io;
use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd};

use rustix::fs::{Mode, OFlags, open};
use rustix::ioctl::{Opcode, Setter, ioctl, opcode};

use crate::error::{Result, TapError};

const TUN_DEVICE: &str = "/dev/net/tun";
const IF_NAME_MAX: usize = libc::IFNAMSIZ;

const IFF_TAP: libc::c_short = 0x0002;
const IFF_NO_PI: libc::c_short = 0x1000;
const IFF_VNET_HDR: libc::c_short = 0x4000;

const TUNSETIFF: Opcode = opcode::write::<libc::c_int>(b'T', 202);
const TUNSETPERSIST: Opcode = opcode::write::<libc::c_int>(b'T', 203);

const PERSIST: libc::c_int = 1;
const NON_PERSIST: libc::c_int = 0;

#[repr(C)]
struct IfReq {
    ifr_name: [u8; IF_NAME_MAX],
    ifr_flags: libc::c_short,
    _padding: [u8; 22],
}

/// Creates a persistent TAP device named `name` with virtio headers enabled.
///
/// # Errors
///
/// Returns [`TapError::InvalidName`] when the name is empty or does not fit `IFNAMSIZ`, and [`TapError::Io`] when the tun
/// device cannot be opened or the `TUNSETIFF` or `TUNSETPERSIST` ioctls fail.
pub fn create(name: &str) -> Result<OwnedFd> {
    let fd = attach(name)?;
    set_persist(fd.as_fd(), PERSIST)?;

    Ok(fd)
}

/// Deletes the persistent TAP device named `name`.
///
/// # Errors
///
/// Returns [`TapError::InvalidName`] when the name is empty or does not fit `IFNAMSIZ`, and [`TapError::Io`] when the tun
/// device cannot be opened or an ioctl fails. Both ioctls also fail when the device cannot be changed: `TUNSETIFF`
/// returns `EBUSY` while another queue holder keeps the device attached, `EPERM` without `CAP_NET_ADMIN`, and `EINVAL`
/// when the name exists but is not a TAP device or was created with different queue flags. The kernel
/// [`std::io::Error`] is kept as the source of [`TapError::Io`].
pub fn delete(name: &str) -> Result<()> {
    let fd = attach(name)?;

    set_persist(fd.as_fd(), NON_PERSIST)
}

fn attach(name: &str) -> Result<OwnedFd> {
    let ifr = IfReq {
        ifr_name: build_ifreq_name(name)?,
        ifr_flags: IFF_TAP | IFF_NO_PI | IFF_VNET_HDR,
        _padding: [0_u8; 22],
    };

    let fd =
        open(TUN_DEVICE, OFlags::RDWR | OFlags::CLOEXEC, Mode::empty()).map_err(io::Error::from)?;

    let set_iff = unsafe {
        // SAFETY: `Setter::new` packages a plain old data `ifreq` for `TUNSETIFF`.
        Setter::<TUNSETIFF, IfReq>::new(ifr)
    };
    unsafe {
        // SAFETY: The opcode and argument layout match the `TUNSETIFF` ioctl contract.
        ioctl(&fd, set_iff)
    }
    .map_err(io::Error::from)?;

    Ok(fd)
}

fn set_persist(fd: BorrowedFd<'_>, persist: libc::c_int) -> Result<()> {
    let set_persist = unsafe {
        // SAFETY: `Setter::new` packages the persistence flag for `TUNSETPERSIST`.
        Setter::<TUNSETPERSIST, libc::c_int>::new(persist)
    };

    unsafe {
        // SAFETY: The opcode and argument layout match the `TUNSETPERSIST` ioctl contract.
        ioctl(fd, set_persist)
    }
    .map_err(io::Error::from)?;

    Ok(())
}

const fn build_ifreq_name(name: &str) -> Result<[u8; IF_NAME_MAX]> {
    let bytes = name.as_bytes();
    let length = bytes.len().saturating_add(1);
    if bytes.is_empty() || length > IF_NAME_MAX {
        return Err(TapError::InvalidName);
    }

    let mut ifr_name = [0_u8; IF_NAME_MAX];
    let (head, _terminator) = ifr_name.split_at_mut(bytes.len());
    head.copy_from_slice(bytes);

    Ok(ifr_name)
}

#[cfg(test)]
mod tests {
    use core::mem::size_of;

    use super::*;

    #[test]
    fn ifreq_matches_kernel_layout() {
        // ARRANGE
        let expected = 40;

        // ACT
        let length = size_of::<IfReq>();

        // ASSERT
        assert_eq!(length, expected);
    }

    #[test]
    fn build_ifreq_name_rejects_names_longer_than_the_kernel_limit() {
        // ARRANGE
        let name = "tap-name-that-is-too-long";

        // ACT
        let result = build_ifreq_name(name);

        // ASSERT
        assert!(matches!(result, Err(TapError::InvalidName)));
    }

    #[test]
    fn build_ifreq_name_rejects_empty_names() {
        // ARRANGE
        let name = "";

        // ACT
        let result = build_ifreq_name(name);

        // ASSERT
        assert!(matches!(result, Err(TapError::InvalidName)));
    }

    #[test]
    fn build_ifreq_name_zero_fills_unused_tail() {
        // ARRANGE
        let name = "tap0";

        // ACT
        let ifr_name = build_ifreq_name(name).expect("valid tap name");

        // ASSERT
        assert_eq!(ifr_name[..4], *b"tap0");
        assert!(ifr_name[4..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn create_rejects_invalid_name_before_touching_the_tun_device() {
        // ARRANGE
        let name = "tap-name-that-is-too-long";

        // ACT
        let result = create(name);

        // ASSERT
        assert!(matches!(result, Err(TapError::InvalidName)));
    }

    #[test]
    fn delete_rejects_invalid_name_before_touching_the_tun_device() {
        // ARRANGE
        let name = "";

        // ACT
        let result = delete(name);

        // ASSERT
        assert!(matches!(result, Err(TapError::InvalidName)));
    }
}
