//! Platform-independent Secure Boot firmware variable status reads.

use super::siglist;
use super::variables::{self, Backend, Id};
use crate::error::Result;

/// Check if running in EFI boot mode.
#[must_use]
pub fn is_boot<B: Backend>(backend: &B) -> bool {
    backend.is_firmware_boot()
}

/// Mount platform firmware variable storage if needed.
///
/// # Errors
///
/// Returns an error if firmware variable storage cannot be prepared.
pub fn mount<B: Backend>(backend: &B) -> Result<bool> {
    backend.ensure_ready()
}

/// Check if platform firmware variable storage is available.
#[must_use]
pub fn is_available<B: Backend>(backend: &B) -> bool {
    backend.is_available()
}

/// Check if Secure Boot is enabled.
///
/// # Errors
///
/// Returns an error if the Secure Boot variable cannot be read.
pub fn secure_boot<B: Backend>(backend: &B) -> Result<bool> {
    read_boolean_variable(backend, &variables::SECURE_BOOT)
}

/// Check if system is in Setup Mode.
///
/// # Errors
///
/// Returns an error if the Setup Mode variable cannot be read.
pub fn setup_mode<B: Backend>(backend: &B) -> Result<bool> {
    if let Some(data) = backend.read_variable(&variables::SETUP_MODE)?
        && !data.is_empty()
    {
        return Ok(data.first().copied() == Some(1));
    }

    Ok(!backend.variable_exists(&variables::PK))
}

/// Get the current Platform Key.
///
/// # Errors
///
/// Returns an error if the Platform Key variable cannot be read or parsed.
pub fn pk<B: Backend>(backend: &B) -> Result<Option<siglist::SignatureDatabase>> {
    signature_database(backend, &variables::PK)
}

/// Get the current Key Exchange Keys.
///
/// # Errors
///
/// Returns an error if the Key Exchange Key variable cannot be read or parsed.
pub fn kek<B: Backend>(backend: &B) -> Result<Option<siglist::SignatureDatabase>> {
    signature_database(backend, &variables::KEK)
}

/// Get the current signature database.
///
/// # Errors
///
/// Returns an error if the signature database variable cannot be read or parsed.
pub fn db<B: Backend>(backend: &B) -> Result<Option<siglist::SignatureDatabase>> {
    signature_database(backend, &variables::DB)
}

/// Read a boolean firmware variable.
fn read_boolean_variable<B: Backend>(backend: &B, id: &Id) -> Result<bool> {
    match backend.read_variable(id)? {
        Some(data) if !data.is_empty() => Ok(data.first().copied() == Some(1)),
        _ => Ok(false),
    }
}

/// Read a signature database with an explicit backend.
fn signature_database<B: Backend>(
    backend: &B,
    id: &Id,
) -> Result<Option<siglist::SignatureDatabase>> {
    match backend.read_variable(id)? {
        Some(data) if !data.is_empty() => Ok(Some(siglist::SignatureDatabase::from_bytes(&data)?)),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use uefi::guid;

    use super::*;
    use crate::efi::guid::{EFI_GLOBAL_VARIABLE, EFI_IMAGE_SECURITY_DATABASE};
    use crate::efi::variables::Update;

    /// In-memory firmware variable backend for status tests.
    #[derive(Default)]
    struct FakeFirmwareVariables {
        firmware_boot: bool,
        available: bool,
        ready: bool,
        variables: Vec<(Id, Vec<u8>)>,
    }

    impl FakeFirmwareVariables {
        /// Create a ready fake firmware variable backend.
        fn ready() -> Self {
            Self {
                firmware_boot: true,
                available: true,
                ready: true,
                variables: Vec::new(),
            }
        }

        /// Add a firmware variable to the fake backend.
        fn with_variable(mut self, id: Id, payload: Vec<u8>) -> Self {
            self.variables.push((id, payload));
            self
        }
    }

    impl Backend for FakeFirmwareVariables {
        /// Return whether the fake system was EFI-booted.
        fn is_firmware_boot(&self) -> bool {
            self.firmware_boot
        }

        /// Return whether the fake firmware variable store is available.
        fn is_available(&self) -> bool {
            self.available
        }

        /// Return whether the fake firmware variable store is ready.
        fn ensure_ready(&self) -> Result<bool> {
            Ok(self.ready)
        }

        /// Return whether the fake variable exists.
        fn variable_exists(&self, id: &Id) -> bool {
            self.variables
                .iter()
                .any(|(stored_id, _payload)| stored_id == id)
        }

        /// Read a fake firmware variable.
        fn read_variable(&self, id: &Id) -> Result<Option<Vec<u8>>> {
            Ok(self
                .variables
                .iter()
                .find(|(stored_id, _payload)| stored_id == id)
                .map(|(_stored_id, payload)| payload.clone()))
        }

        /// Ignore fake firmware variable writes.
        fn write_variable(&self, _update: Update<'_>) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn readiness_methods_follow_backend_state() -> Result<()> {
        // ARRANGE
        let backend = FakeFirmwareVariables::ready();

        // ACT & ASSERT
        assert!(is_boot(&backend));
        assert!(is_available(&backend));
        assert!(mount(&backend)?);

        Ok(())
    }

    #[test]
    fn get_secure_boot_and_setup_mode_follow_variable_contents() -> Result<()> {
        // ARRANGE
        let backend = FakeFirmwareVariables::ready()
            .with_variable(variables::SECURE_BOOT, vec![1])
            .with_variable(variables::SETUP_MODE, vec![0]);

        // ACT
        let secure_boot = secure_boot(&backend)?;
        let setup_mode = setup_mode(&backend)?;

        // ASSERT
        assert!(secure_boot);
        assert!(!setup_mode);

        Ok(())
    }

    #[test]
    fn get_setup_mode_falls_back_to_pk_presence() -> Result<()> {
        // ARRANGE
        let with_pk = FakeFirmwareVariables::ready().with_variable(variables::PK, Vec::new());
        let without_pk = FakeFirmwareVariables::ready();

        // ACT
        let setup_with_pk = setup_mode(&with_pk)?;
        let setup_without_pk = setup_mode(&without_pk)?;

        // ASSERT
        assert!(!setup_with_pk);
        assert!(setup_without_pk);

        Ok(())
    }

    #[test]
    fn get_pk_kek_and_db_parse_signature_databases() -> Result<()> {
        // ARRANGE
        let owner = guid!("12345678-1234-1234-1234-123456789abc");
        let siglist = siglist::build_x509(&owner, b"cert-bytes")?;
        let backend = FakeFirmwareVariables::ready()
            .with_variable(variables::PK, siglist.clone())
            .with_variable(variables::KEK, siglist.clone())
            .with_variable(variables::DB, siglist);

        // ACT
        let pk = pk(&backend)?;
        let kek = kek(&backend)?;
        let db = db(&backend)?;

        // ASSERT
        assert_eq!(pk.map_or(0, |database| database.len()), 1);
        assert_eq!(kek.map_or(0, |database| database.len()), 1);
        assert_eq!(db.map_or(0, |database| database.len()), 1);

        Ok(())
    }

    #[test]
    fn firmware_variable_ids_use_expected_namespaces() {
        // ACT & ASSERT
        assert_eq!(variables::PK.name(), "PK");
        assert_eq!(variables::PK.vendor_guid(), &EFI_GLOBAL_VARIABLE);
        assert_eq!(variables::DB.name(), "db");
        assert_eq!(variables::DB.vendor_guid(), &EFI_IMAGE_SECURITY_DATABASE);
    }
}
