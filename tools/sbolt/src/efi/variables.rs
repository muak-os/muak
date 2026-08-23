//! Firmware variable backend abstraction.

use uefi::Guid;

use super::guid::{EFI_GLOBAL_VARIABLE, EFI_IMAGE_SECURITY_DATABASE};
use crate::error::Result;

/// Secure Boot enabled flag variable.
pub const SECURE_BOOT: Id = Id::new("SecureBoot", EFI_GLOBAL_VARIABLE);

/// Setup Mode flag variable.
pub const SETUP_MODE: Id = Id::new("SetupMode", EFI_GLOBAL_VARIABLE);

/// Platform Key variable.
pub const PK: Id = Id::new("PK", EFI_GLOBAL_VARIABLE);

/// Key Exchange Key variable.
pub const KEK: Id = Id::new("KEK", EFI_GLOBAL_VARIABLE);

/// Signature database variable.
pub const DB: Id = Id::new("db", EFI_IMAGE_SECURITY_DATABASE);

/// Identifies a UEFI firmware variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Id {
    name: &'static str,
    vendor_guid: Guid,
}

impl Id {
    /// Create a firmware variable identifier.
    #[must_use]
    pub const fn new(name: &'static str, vendor_guid: Guid) -> Self {
        Self { name, vendor_guid }
    }

    /// Return the firmware variable name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Return the firmware variable vendor GUID.
    #[must_use]
    pub const fn vendor_guid(&self) -> &Guid {
        &self.vendor_guid
    }
}

/// Authenticated firmware variable update payload.
pub struct Update<'a> {
    id: Id,
    payload: &'a [u8],
}

impl<'a> Update<'a> {
    /// Create an authenticated firmware variable update.
    #[must_use]
    pub const fn new(id: Id, payload: &'a [u8]) -> Self {
        Self { id, payload }
    }

    /// Return the target firmware variable identifier.
    #[must_use]
    pub const fn id(&self) -> &Id {
        &self.id
    }

    /// Return the encoded update payload.
    #[must_use]
    pub const fn payload(&self) -> &'a [u8] {
        self.payload
    }
}

/// Backend used to access firmware variables on the host platform.
pub trait Backend {
    /// Return whether the system was booted through EFI firmware.
    fn is_firmware_boot(&self) -> bool;

    /// Return whether firmware variable storage is currently available.
    fn is_available(&self) -> bool;

    /// Prepare firmware variable storage and report whether it is ready.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend cannot prepare firmware variable storage.
    fn ensure_ready(&self) -> Result<bool>;

    /// Return whether a firmware variable exists.
    fn variable_exists(&self, id: &Id) -> bool;

    /// Read a firmware variable payload without platform storage metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend cannot read the firmware variable.
    fn read_variable(&self, id: &Id) -> Result<Option<Vec<u8>>>;

    /// Write an authenticated firmware variable update payload.
    ///
    /// # Errors
    ///
    /// Returns an error if the backend cannot write the firmware variable.
    fn write_variable(&self, update: Update<'_>) -> Result<()>;
}
