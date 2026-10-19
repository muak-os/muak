//! UEFI variable identifiers and storage.

use uefi::{Guid, guid};

use crate::error::Result;

/// Namespace for `PK`, `KEK`, `SetupMode`, and `SecureBoot`.
pub const EFI_GLOBAL_VARIABLE: Guid = guid!("8be4df61-93ca-11d2-aa0d-00e098032b8c");

/// Namespace for `db` and `dbx`.
pub const EFI_IMAGE_SECURITY_DATABASE: Guid = guid!("d719b2cb-3d3a-4596-a3bc-dad00e67656f");

/// Storage for UEFI firmware variables.
pub trait VariableStore {
    /// Returns whether the system was booted through EFI firmware.
    fn is_boot(&self) -> bool;

    /// Returns whether firmware variable storage is currently available.
    fn is_available(&self) -> bool;

    /// Prepares firmware variable storage and reports whether it is ready.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot prepare firmware variable storage.
    fn prepare(&self) -> Result<bool>;

    /// Returns whether a firmware variable exists.
    fn exists(&self, id: &Id) -> bool;

    /// Reads a firmware variable payload without platform storage metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot read the firmware variable.
    fn read(&self, id: &Id) -> Result<Option<Vec<u8>>>;

    /// Writes a firmware variable update.
    ///
    /// # Errors
    ///
    /// Returns an error if the store cannot write the firmware variable.
    fn write(&self, update: Update<'_>) -> Result<()>;
}

/// Reads a firmware variable from `store` and decodes it as a UTF-16 string.
///
/// # Errors
///
/// Returns an error if the store cannot read the firmware variable.
pub fn read_string<S: VariableStore>(store: &S, id: &Id) -> Result<Option<String>> {
    Ok(store.read(id)?.map(|bytes| decode_string(&bytes)))
}

fn decode_string(bytes: &[u8]) -> String {
    let mut units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|chunk| u16::from_le_bytes(*chunk))
        .collect();

    while units.last() == Some(&0) {
        units.pop();
    }

    String::from_utf16_lossy(&units)
}

/// Identifies a UEFI firmware variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Id {
    name: &'static str,
    vendor_guid: Guid,
}

impl Id {
    /// Creates a firmware variable identifier.
    #[must_use]
    pub const fn new(name: &'static str, vendor_guid: Guid) -> Self {
        Self { name, vendor_guid }
    }

    /// Returns the firmware variable name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Returns the firmware variable vendor GUID.
    #[must_use]
    pub const fn vendor_guid(&self) -> &Guid {
        &self.vendor_guid
    }
}

/// Firmware variable update.
pub struct Update<'a> {
    id: Id,
    attributes: u32,
    data: &'a [u8],
}

impl<'a> Update<'a> {
    /// Creates a firmware variable update.
    #[must_use]
    pub const fn new(id: Id, attributes: u32, data: &'a [u8]) -> Self {
        Self {
            id,
            attributes,
            data,
        }
    }

    /// Returns the target firmware variable identifier.
    #[must_use]
    pub const fn id(&self) -> &Id {
        &self.id
    }

    /// Returns the EFI variable attribute bits.
    #[must_use]
    pub const fn attributes(&self) -> u32 {
        self.attributes
    }

    /// Returns the variable data without the `efivarfs` attribute header.
    #[must_use]
    pub const fn data(&self) -> &'a [u8] {
        self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_string_trims_trailing_nul() {
        // ARRANGE
        let mut bytes: Vec<u8> = "Muak".encode_utf16().flat_map(u16::to_le_bytes).collect();
        bytes.extend_from_slice(&[0, 0]);

        // ACT
        let decoded = decode_string(&bytes);

        // ASSERT
        assert_eq!(decoded, "Muak");
    }

    #[test]
    fn decode_string_ignores_trailing_partial_unit() {
        // ARRANGE
        let mut bytes: Vec<u8> = "Hi".encode_utf16().flat_map(u16::to_le_bytes).collect();
        bytes.push(0);

        // ACT
        let decoded = decode_string(&bytes);

        // ASSERT
        assert_eq!(decoded, "Hi");
    }

    #[test]
    fn decode_string_of_empty_bytes_is_empty() {
        // ARRANGE
        let bytes: [u8; 0] = [];

        // ACT
        let decoded = decode_string(&bytes);

        // ASSERT
        assert!(decoded.is_empty(), "empty bytes decode to an empty string");
    }
}
