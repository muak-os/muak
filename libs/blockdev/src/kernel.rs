//! Kernel view of the block devices attached to the system.

use crate::error::Result;
use crate::mounts::{Mount, Swap};
use crate::types::Disk;

/// Kernel view of the block devices attached to the system.
pub trait Kernel {
    /// Returns the physical disks the kernel reports.
    ///
    /// # Errors
    ///
    /// Returns an error when the block tree is missing or cannot be read.
    fn disks(&self) -> Result<Vec<Disk>>;

    /// Returns the partition device paths carrying `partname`.
    fn find_by_partname(&self, partname: &str) -> Vec<String>;

    /// Returns the device-mapper names of the holders of `device`.
    ///
    /// # Errors
    ///
    /// Returns an error when the holders directory exists but cannot be read.
    fn holders(&self, device: &str) -> Result<Vec<String>>;

    /// Returns the kernel mount table.
    fn mounts(&self) -> Vec<Mount>;

    /// Returns the name of the disk carrying `partition_device`.
    fn parent_disk(&self, partition_device: &str) -> Option<String>;

    /// Returns the size in bytes of the device behind `fd`.
    ///
    /// # Errors
    ///
    /// Returns an error when the descriptor lookup fails, the descriptor reports
    /// a negative size, or a block device rejects the size ioctl.
    fn size<Fd: std::os::fd::AsFd>(&self, fd: &Fd) -> Result<u64>;

    /// Returns the kernel swap table.
    ///
    /// # Errors
    ///
    /// Returns an error when the table exists but cannot be read.
    fn swaps(&self) -> Result<Vec<Swap>>;
}
