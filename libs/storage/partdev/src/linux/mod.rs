//! Linux partition table re-reads through the `BLKRRPART` ioctl.

use std::fs::File;

use rustix::ioctl::{NoArg, Opcode, ioctl, opcode};

use crate::error::Result;
use crate::kernel::Kernel;

// The `BLKRRPART` ioctl: `_IO(0x12, 95)`.
const BLKRRPART: Opcode = opcode::none(0x12, 95);

/// Linux implementation of the kernel partition table operations.
#[derive(Debug, Clone, Default)]
pub struct Linux;

impl Linux {
    /// Creates the Linux kernel handle.
    #[must_use = "the handle is the only way to reach the kernel"]
    pub const fn new() -> Self {
        Self
    }
}

impl Kernel for Linux {
    fn rescan(&self, device: &str) -> Result<()> {
        let file = File::open(device)?;

        // SAFETY: `BLKRRPART` is a valid no-argument opcode for block devices.
        let request = unsafe { NoArg::<BLKRRPART>::new() };
        // SAFETY: the descriptor is open and the opcode takes no argument.
        unsafe { ioctl(&file, request) }.map_err(|errno| std::io::Error::from(errno).into())
    }

    fn size(&self, file: &File) -> Result<u64> {
        Ok(blockdev::device::size(file)?)
    }
}
