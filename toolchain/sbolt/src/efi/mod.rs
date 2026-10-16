//! EFI types and firmware variable operations.

pub mod authvar;
pub mod enrollment;
pub mod guid;
pub mod siglist;
pub mod time;

use crate::error::Result;
use crate::keys::hierarchy;

/// Enroll the complete key hierarchy into UEFI firmware.
///
/// # Errors
///
/// Returns an error if firmware variable storage is unavailable, the system is
/// not in Setup Mode, certificate encoding fails, or any variable write fails.
pub fn enroll(hierarchy: &hierarchy::Bundle) -> Result<()> {
    enrollment::enroll(&firmware::system(), hierarchy)
}

/// Remove the complete key hierarchy from UEFI firmware and disable enforcement.
///
/// # Errors
///
/// Returns an error if firmware variable storage is unavailable, the Platform
/// Key cannot be deleted, or the firmware still reports an enrolled Platform
/// Key afterwards.
pub fn unenroll(hierarchy: &hierarchy::Bundle) -> Result<()> {
    enrollment::unenroll(&firmware::system(), hierarchy)
}
