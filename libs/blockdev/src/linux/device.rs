//! Block device path helpers and size probing.

use std::io::ErrorKind;
use std::os::fd::AsFd;

use rustix::fs::FileType;
use rustix::ioctl::{Getter, Opcode, ioctl, opcode};

use crate::error::Result;

// The `BLKGETSIZE64` ioctl: `_IOR(0x12, 114, size_t)`.
const BLKGETSIZE64: Opcode = opcode::read::<u64>(0x12, 114);

/// Returns the size in bytes of the block device or file behind `fd`.
///
/// # Errors
///
/// Returns an error when the descriptor lookup fails, the descriptor reports a
/// negative size, or a block device rejects the size ioctl.
pub(crate) fn size<Fd: AsFd>(fd: &Fd) -> Result<u64> {
    let stat = rustix::fs::fstat(fd).map_err(std::io::Error::from)?;
    if !FileType::from_raw_mode(stat.st_mode).is_block_device() {
        return u64::try_from(stat.st_size)
            .map_err(|_error| std::io::Error::from(ErrorKind::InvalidData).into());
    }

    // SAFETY: `BLKGETSIZE64` writes a `u64` and `Getter` carries the matching type.
    let getter = unsafe { Getter::<BLKGETSIZE64, u64>::new() };
    // SAFETY: the descriptor is valid and the ioctl argument type matches the opcode.
    unsafe { ioctl(fd.as_fd(), getter) }.map_err(|errno| std::io::Error::from(errno).into())
}
