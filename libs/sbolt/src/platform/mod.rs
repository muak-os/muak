//! Platform firmware variable backends.

#[cfg(all(feature = "linux", target_os = "linux"))]
pub(crate) mod linux;

use uefi::Guid;

use crate::efi::guid::{EFI_GLOBAL_VARIABLE, EFI_IMAGE_SECURITY_DATABASE};
use crate::error::Result;

/// Secure Boot enabled flag variable.
pub(crate) const SECURE_BOOT_VARIABLE: FirmwareVariableId =
    FirmwareVariableId::new("SecureBoot", EFI_GLOBAL_VARIABLE);

/// Setup Mode flag variable.
pub(crate) const SETUP_MODE_VARIABLE: FirmwareVariableId =
    FirmwareVariableId::new("SetupMode", EFI_GLOBAL_VARIABLE);

/// Platform Key variable.
pub(crate) const PK_VARIABLE: FirmwareVariableId =
    FirmwareVariableId::new("PK", EFI_GLOBAL_VARIABLE);

/// Key Exchange Key variable.
pub(crate) const KEK_VARIABLE: FirmwareVariableId =
    FirmwareVariableId::new("KEK", EFI_GLOBAL_VARIABLE);

/// Signature database variable.
pub(crate) const DB_VARIABLE: FirmwareVariableId =
    FirmwareVariableId::new("db", EFI_IMAGE_SECURITY_DATABASE);

/// Identifies a UEFI firmware variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FirmwareVariableId {
    name: &'static str,
    vendor_guid: Guid,
}

impl FirmwareVariableId {
    /// Create a firmware variable identifier.
    pub(crate) const fn new(name: &'static str, vendor_guid: Guid) -> Self {
        Self { name, vendor_guid }
    }

    /// Return the firmware variable name.
    #[must_use]
    pub(crate) const fn name(&self) -> &'static str {
        self.name
    }

    /// Return the firmware variable vendor GUID.
    #[must_use]
    pub(crate) const fn vendor_guid(&self) -> &Guid {
        &self.vendor_guid
    }
}

/// Authenticated firmware variable update payload.
pub(crate) struct FirmwareVariableUpdate<'a> {
    id: FirmwareVariableId,
    payload: &'a [u8],
}

impl<'a> FirmwareVariableUpdate<'a> {
    /// Create an authenticated firmware variable update.
    pub(crate) const fn new(id: FirmwareVariableId, payload: &'a [u8]) -> Self {
        Self { id, payload }
    }

    /// Return the target firmware variable identifier.
    #[must_use]
    pub(crate) const fn id(&self) -> &FirmwareVariableId {
        &self.id
    }

    /// Return the encoded update payload.
    #[must_use]
    pub(crate) const fn payload(&self) -> &'a [u8] {
        self.payload
    }
}

/// Backend used to access firmware variables on the host platform.
pub(crate) trait FirmwareVariableBackend {
    /// Return whether the system was booted through EFI firmware.
    fn is_firmware_boot(&self) -> bool;

    /// Return whether firmware variable storage is currently available.
    fn is_available(&self) -> bool;

    /// Prepare firmware variable storage and report whether it is ready.
    fn ensure_ready(&self) -> Result<bool>;

    /// Return whether a firmware variable exists.
    fn variable_exists(&self, id: &FirmwareVariableId) -> bool;

    /// Read a firmware variable payload without platform storage metadata.
    fn read_variable(&self, id: &FirmwareVariableId) -> Result<Option<Vec<u8>>>;

    /// Write an authenticated firmware variable update payload.
    fn write_variable(&self, update: FirmwareVariableUpdate<'_>) -> Result<()>;
}
