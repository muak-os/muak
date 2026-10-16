//! Platform firmware data.

use crate::esrt::RawEntry;

/// Platform firmware data beyond firmware variables.
pub trait Platform {
    /// Returns an SMBIOS firmware attribute, when the platform exposes SMBIOS.
    fn smbios(&self, attribute: SmbiosAttribute) -> Option<String>;

    /// Returns a device tree property, when the platform exposes one.
    fn devicetree(&self, property: DevicetreeProperty) -> Option<String>;

    /// Returns the ESRT entries exposed by the firmware.
    fn esrt(&self) -> Vec<RawEntry>;
}

/// SMBIOS type 0 attribute requested from a [`Platform`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmbiosAttribute {
    /// Firmware vendor.
    Vendor,
    /// Firmware version.
    Version,
    /// Firmware release date.
    Date,
}

/// Device tree property requested from a [`Platform`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevicetreeProperty {
    /// Board model string.
    Model,
    /// Board compatible list.
    Compatible,
}
