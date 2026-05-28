//! EFI types and firmware variable operations.

pub mod authvar;
pub mod guid;
mod service;
pub mod siglist;
pub mod time;

#[cfg(all(feature = "linux", target_os = "linux"))]
use crate::platform::linux::LinuxFirmwareVariables as FirmwareVariables;

#[cfg(not(all(feature = "linux", target_os = "linux")))]
compile_error!(
    "sbolt EFI firmware variable APIs currently require the `linux` feature on a Linux target"
);

use crate::error::Result;
use crate::keys::hierarchy;

/// Check if running in EFI boot mode.
#[must_use]
pub fn is_boot() -> bool {
    service::is_boot(&FirmwareVariables::new())
}

/// Mount platform firmware variable storage if needed.
///
/// # Errors
///
/// Returns an error if firmware variable storage cannot be prepared.
pub fn mount() -> Result<bool> {
    service::mount(&FirmwareVariables::new())
}

/// Check if platform firmware variable storage is available.
#[must_use]
pub fn is_available() -> bool {
    service::is_available(&FirmwareVariables::new())
}

/// Check if Secure Boot is enabled.
///
/// # Errors
///
/// Returns an error if the Secure Boot variable cannot be read.
pub fn secure_boot() -> Result<bool> {
    service::secure_boot(&FirmwareVariables::new())
}

/// Check if system is in Setup Mode.
///
/// # Errors
///
/// Returns an error if the Setup Mode variable cannot be read.
pub fn setup_mode() -> Result<bool> {
    service::setup_mode(&FirmwareVariables::new())
}

/// Get the current Platform Key.
///
/// # Errors
///
/// Returns an error if the Platform Key variable cannot be read or parsed.
pub fn pk() -> Result<Option<siglist::SignatureDatabase>> {
    service::pk(&FirmwareVariables::new())
}

/// Get the current Key Exchange Keys.
///
/// # Errors
///
/// Returns an error if the Key Exchange Key variable cannot be read or parsed.
pub fn kek() -> Result<Option<siglist::SignatureDatabase>> {
    service::kek(&FirmwareVariables::new())
}

/// Get the current signature database.
///
/// # Errors
///
/// Returns an error if the signature database variable cannot be read or parsed.
pub fn db() -> Result<Option<siglist::SignatureDatabase>> {
    service::db(&FirmwareVariables::new())
}

/// Enroll the complete key hierarchy into UEFI firmware.
///
/// # Errors
///
/// Returns an error if firmware variable storage is unavailable, the system is
/// not in Setup Mode, certificate encoding fails, or any variable write fails.
pub fn enroll(hierarchy: &hierarchy::Bundle) -> Result<()> {
    service::enroll(&FirmwareVariables::new(), hierarchy)
}
