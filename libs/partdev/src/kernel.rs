//! Partition table operations the running kernel must provide.

use std::fs::File;

use crate::error::Result;

/// Kernel side of a partition table: device sizing and table re-reads.
pub trait Kernel {
    /// Asks the kernel to re-read the partition table of `device`.
    ///
    /// # Errors
    ///
    /// Returns an error when the device cannot be opened or the kernel
    /// rejects the re-read.
    fn rescan(&self, device: &str) -> Result<()>;

    /// Returns the size in bytes of the device behind `file`.
    ///
    /// # Errors
    ///
    /// Returns an error when the size cannot be determined.
    fn size(&self, file: &File) -> Result<u64>;
}

/// Asks the active kernel to re-read the partition table of `device`.
///
/// # Errors
///
/// Returns an error when the device cannot be opened or the kernel rejects
/// the re-read.
pub fn rescan(device: &str) -> Result<()> {
    crate::handle().rescan(device)
}
