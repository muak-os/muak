//! Physical disk inventory read from the kernel.

use crate::error::Result;
use crate::handle;
use crate::kernel::Kernel as _;
use crate::types::Disk;

/// Lists the physical disks reported by the running kernel.
///
/// # Errors
///
/// Returns an error when the block tree is missing or cannot be read, which
/// usually means sysfs is not mounted.
pub fn list() -> Result<Vec<Disk>> {
    handle().disks()
}
