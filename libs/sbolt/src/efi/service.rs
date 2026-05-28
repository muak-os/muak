//! Platform-independent Secure Boot firmware variable operations.

use der::Encode as _;
use uefi::runtime::VariableAttributes;
use x509_cert::Certificate;

use super::authvar;
use super::siglist;
use crate::error::{Result, SboltError};
use crate::keys::hierarchy;
use crate::keys::rsa2048;
use crate::platform::{
    DB_VARIABLE, FirmwareVariableBackend, FirmwareVariableId, FirmwareVariableUpdate, KEK_VARIABLE,
    PK_VARIABLE, SECURE_BOOT_VARIABLE, SETUP_MODE_VARIABLE,
};

/// Signature authority for an authenticated firmware variable update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SigningAuthority {
    PlatformKey,
    KeyExchangeKey,
}

/// Check if running in EFI boot mode.
pub(super) fn is_boot<B: FirmwareVariableBackend>(backend: &B) -> bool {
    backend.is_firmware_boot()
}

/// Mount platform firmware variable storage if needed.
///
/// # Errors
///
/// Returns an error if firmware variable storage cannot be prepared.
pub(super) fn mount<B: FirmwareVariableBackend>(backend: &B) -> Result<bool> {
    backend.ensure_ready()
}

/// Check if platform firmware variable storage is available.
pub(super) fn is_available<B: FirmwareVariableBackend>(backend: &B) -> bool {
    backend.is_available()
}

/// Check if Secure Boot is enabled.
///
/// # Errors
///
/// Returns an error if the Secure Boot variable cannot be read.
pub(super) fn secure_boot<B: FirmwareVariableBackend>(backend: &B) -> Result<bool> {
    read_boolean_variable(backend, &SECURE_BOOT_VARIABLE)
}

/// Check if system is in Setup Mode.
///
/// # Errors
///
/// Returns an error if the Setup Mode variable cannot be read.
pub(super) fn setup_mode<B: FirmwareVariableBackend>(backend: &B) -> Result<bool> {
    if let Some(data) = backend.read_variable(&SETUP_MODE_VARIABLE)?
        && !data.is_empty()
    {
        return Ok(data.first().copied() == Some(1));
    }

    Ok(!backend.variable_exists(&PK_VARIABLE))
}

/// Get the current Platform Key.
///
/// # Errors
///
/// Returns an error if the Platform Key variable cannot be read or parsed.
pub(super) fn pk<B: FirmwareVariableBackend>(
    backend: &B,
) -> Result<Option<siglist::SignatureDatabase>> {
    signature_database(backend, &PK_VARIABLE)
}

/// Get the current Key Exchange Keys.
///
/// # Errors
///
/// Returns an error if the Key Exchange Key variable cannot be read or parsed.
pub(super) fn kek<B: FirmwareVariableBackend>(
    backend: &B,
) -> Result<Option<siglist::SignatureDatabase>> {
    signature_database(backend, &KEK_VARIABLE)
}

/// Get the current signature database.
///
/// # Errors
///
/// Returns an error if the signature database variable cannot be read or parsed.
pub(super) fn db<B: FirmwareVariableBackend>(
    backend: &B,
) -> Result<Option<siglist::SignatureDatabase>> {
    signature_database(backend, &DB_VARIABLE)
}

/// Enroll the complete key hierarchy into UEFI firmware.
///
/// # Errors
///
/// Returns an error if firmware variable storage is unavailable, the system is
/// not in Setup Mode, certificate encoding fails, or any variable write fails.
pub(super) fn enroll<B: FirmwareVariableBackend>(
    backend: &B,
    hierarchy: &hierarchy::Bundle,
) -> Result<()> {
    if !backend.ensure_ready()? {
        return Err(SboltError::EfiVar("efivarfs not available".into()));
    }

    if !setup_mode(backend)? {
        return Err(SboltError::EfiVar(
            "system is not in Setup Mode, cannot enroll keys".into(),
        ));
    }

    let db_sigdb = certificate_database(
        &hierarchy.owner_guid,
        &hierarchy.db.certificate,
        "encode db cert",
    )?;
    let kek_sigdb = certificate_database(
        &hierarchy.owner_guid,
        &hierarchy.kek.certificate,
        "encode kek cert",
    )?;
    let pk_sigdb = certificate_database(
        &hierarchy.owner_guid,
        &hierarchy.pk.certificate,
        "encode pk cert",
    )?;

    write_signed_variable(
        backend,
        DB_VARIABLE,
        &db_sigdb.to_bytes(),
        hierarchy,
        SigningAuthority::KeyExchangeKey,
    )
    .map_err(|e| SboltError::EfiVar(format!("failed to enroll db: {e}")))?;
    write_signed_variable(
        backend,
        KEK_VARIABLE,
        &kek_sigdb.to_bytes(),
        hierarchy,
        SigningAuthority::PlatformKey,
    )
    .map_err(|e| SboltError::EfiVar(format!("failed to enroll KEK: {e}")))?;
    write_signed_variable(
        backend,
        PK_VARIABLE,
        &pk_sigdb.to_bytes(),
        hierarchy,
        SigningAuthority::PlatformKey,
    )
    .map_err(|e| SboltError::EfiVar(format!("failed to enroll PK: {e}")))?;

    Ok(())
}

/// Read a boolean firmware variable.
fn read_boolean_variable<B: FirmwareVariableBackend>(
    backend: &B,
    id: &FirmwareVariableId,
) -> Result<bool> {
    match backend.read_variable(id)? {
        Some(data) if !data.is_empty() => Ok(data.first().copied() == Some(1)),
        _ => Ok(false),
    }
}

/// Read a signature database with an explicit backend.
fn signature_database<B: FirmwareVariableBackend>(
    backend: &B,
    id: &FirmwareVariableId,
) -> Result<Option<siglist::SignatureDatabase>> {
    match backend.read_variable(id)? {
        Some(data) if !data.is_empty() => Ok(Some(siglist::SignatureDatabase::from_bytes(&data)?)),
        _ => Ok(None),
    }
}

/// Build a signature database containing a certificate.
fn certificate_database(
    owner_guid: &uefi::Guid,
    certificate: &Certificate,
    encode_context: &str,
) -> Result<siglist::SignatureDatabase> {
    let cert_der = certificate
        .to_der()
        .map_err(|e| SboltError::EfiVar(format!("{encode_context}: {e}")))?;
    let mut sigdb = siglist::SignatureDatabase::new();
    sigdb.add_x509(owner_guid, &cert_der)?;

    Ok(sigdb)
}

/// Write an authenticated firmware variable update.
fn write_signed_variable<B: FirmwareVariableBackend>(
    backend: &B,
    id: FirmwareVariableId,
    content: &[u8],
    hierarchy: &hierarchy::Bundle,
    authority: SigningAuthority,
) -> Result<()> {
    let (signer, certificate) = signer_and_certificate(hierarchy, authority);
    let payload = authvar::sign(
        id.name(),
        id.vendor_guid(),
        authenticated_variable_attributes(),
        content,
        signer,
        certificate,
    )?;

    backend.write_variable(FirmwareVariableUpdate::new(id, &payload))
}

/// Return the signer and certificate for an update authority.
fn signer_and_certificate(
    hierarchy: &hierarchy::Bundle,
    authority: SigningAuthority,
) -> (&rsa2048::Signer, &Certificate) {
    match authority {
        SigningAuthority::PlatformKey => (&hierarchy.pk.signer, &hierarchy.pk.certificate),
        SigningAuthority::KeyExchangeKey => (&hierarchy.kek.signer, &hierarchy.kek.certificate),
    }
}

/// Return attributes for time-based authenticated Secure Boot variables.
fn authenticated_variable_attributes() -> VariableAttributes {
    VariableAttributes::NON_VOLATILE
        .union(VariableAttributes::BOOTSERVICE_ACCESS)
        .union(VariableAttributes::RUNTIME_ACCESS)
        .union(VariableAttributes::TIME_BASED_AUTHENTICATED_WRITE_ACCESS)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use uefi::guid;

    use super::*;
    use crate::efi::guid::{EFI_GLOBAL_VARIABLE, EFI_IMAGE_SECURITY_DATABASE};

    /// In-memory firmware variable backend for service tests.
    #[derive(Default)]
    struct FakeFirmwareVariables {
        firmware_boot: bool,
        available: bool,
        ready: bool,
        variables: Vec<(FirmwareVariableId, Vec<u8>)>,
        writes: RefCell<Vec<(FirmwareVariableId, Vec<u8>)>>,
    }

    impl FakeFirmwareVariables {
        /// Create a ready fake firmware variable backend.
        fn ready() -> Self {
            Self {
                firmware_boot: true,
                available: true,
                ready: true,
                variables: Vec::new(),
                writes: RefCell::new(Vec::new()),
            }
        }

        /// Add a firmware variable to the fake backend.
        fn with_variable(mut self, id: FirmwareVariableId, payload: Vec<u8>) -> Self {
            self.variables.push((id, payload));
            self
        }

        /// Return recorded firmware variable writes.
        fn writes(&self) -> Vec<(FirmwareVariableId, Vec<u8>)> {
            self.writes.borrow().clone()
        }
    }

    impl FirmwareVariableBackend for FakeFirmwareVariables {
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
        fn variable_exists(&self, id: &FirmwareVariableId) -> bool {
            self.variables
                .iter()
                .any(|(stored_id, _payload)| stored_id == id)
        }

        /// Read a fake firmware variable.
        fn read_variable(&self, id: &FirmwareVariableId) -> Result<Option<Vec<u8>>> {
            Ok(self
                .variables
                .iter()
                .find(|(stored_id, _payload)| stored_id == id)
                .map(|(_stored_id, payload)| payload.clone()))
        }

        /// Record a fake firmware variable write.
        fn write_variable(&self, update: FirmwareVariableUpdate<'_>) -> Result<()> {
            self.writes
                .borrow_mut()
                .push((*update.id(), update.payload().to_vec()));

            Ok(())
        }
    }

    #[test]
    fn get_secure_boot_and_setup_mode_follow_variable_contents() -> Result<()> {
        // ARRANGE
        let backend = FakeFirmwareVariables::ready()
            .with_variable(SECURE_BOOT_VARIABLE, vec![1])
            .with_variable(SETUP_MODE_VARIABLE, vec![0]);

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
        let with_pk = FakeFirmwareVariables::ready().with_variable(PK_VARIABLE, Vec::new());
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
            .with_variable(PK_VARIABLE, siglist.clone())
            .with_variable(KEK_VARIABLE, siglist.clone())
            .with_variable(DB_VARIABLE, siglist);

        // ACT
        let pk = signature_database(&backend, &PK_VARIABLE)?;
        let kek = signature_database(&backend, &KEK_VARIABLE)?;
        let db = signature_database(&backend, &DB_VARIABLE)?;

        // ASSERT
        assert_eq!(pk.map_or(0, |database| database.len()), 1);
        assert_eq!(kek.map_or(0, |database| database.len()), 1);
        assert_eq!(db.map_or(0, |database| database.len()), 1);

        Ok(())
    }

    #[test]
    fn enroll_keys_writes_db_kek_and_pk_in_order() -> Result<()> {
        // ARRANGE
        let backend = FakeFirmwareVariables::ready().with_variable(SETUP_MODE_VARIABLE, vec![1]);
        let hierarchy = hierarchy::Bundle::generate("Enroll Success")?;
        let db_sigdb = certificate_database(
            &hierarchy.owner_guid,
            &hierarchy.db.certificate,
            "encode db cert",
        )?;
        let kek_sigdb = certificate_database(
            &hierarchy.owner_guid,
            &hierarchy.kek.certificate,
            "encode kek cert",
        )?;
        let pk_sigdb = certificate_database(
            &hierarchy.owner_guid,
            &hierarchy.pk.certificate,
            "encode pk cert",
        )?;

        // ACT
        enroll(&backend, &hierarchy)?;
        let writes = backend.writes();

        // ASSERT
        assert_eq!(writes.len(), 3);
        assert_eq!(writes.first().map(|(id, _payload)| *id), Some(DB_VARIABLE));
        assert_eq!(writes.get(1).map(|(id, _payload)| *id), Some(KEK_VARIABLE));
        assert_eq!(writes.get(2).map(|(id, _payload)| *id), Some(PK_VARIABLE));
        assert!(
            writes
                .first()
                .map(|(_id, payload)| payload.ends_with(&db_sigdb.to_bytes()))
                .unwrap_or(false)
        );
        assert!(
            writes
                .get(1)
                .map(|(_id, payload)| payload.ends_with(&kek_sigdb.to_bytes()))
                .unwrap_or(false)
        );
        assert!(
            writes
                .get(2)
                .map(|(_id, payload)| payload.ends_with(&pk_sigdb.to_bytes()))
                .unwrap_or(false)
        );

        Ok(())
    }

    #[test]
    fn enroll_keys_rejects_unavailable_backend() -> Result<()> {
        // ARRANGE
        let backend = FakeFirmwareVariables {
            ready: false,
            ..FakeFirmwareVariables::ready()
        };
        let hierarchy = hierarchy::Bundle::generate("Enroll Unavailable")?;

        // ACT
        let result = enroll(&backend, &hierarchy);

        // ASSERT
        assert!(result.is_err());
        assert!(backend.writes().is_empty());

        Ok(())
    }

    #[test]
    fn enroll_keys_rejects_non_setup_mode() -> Result<()> {
        // ARRANGE
        let backend = FakeFirmwareVariables::ready()
            .with_variable(SETUP_MODE_VARIABLE, vec![0])
            .with_variable(PK_VARIABLE, vec![1]);
        let hierarchy = hierarchy::Bundle::generate("Enroll No Setup")?;

        // ACT
        let result = enroll(&backend, &hierarchy);

        // ASSERT
        assert!(result.is_err());
        assert!(backend.writes().is_empty());

        Ok(())
    }

    #[test]
    fn authenticated_variable_attributes_match_secure_boot_requirements() {
        // ARRANGE
        let required = VariableAttributes::NON_VOLATILE
            .union(VariableAttributes::BOOTSERVICE_ACCESS)
            .union(VariableAttributes::RUNTIME_ACCESS)
            .union(VariableAttributes::TIME_BASED_AUTHENTICATED_WRITE_ACCESS);

        // ACT
        let actual = authenticated_variable_attributes();

        // ASSERT
        assert_eq!(actual, required);
    }

    #[test]
    fn firmware_variable_ids_use_expected_namespaces() {
        // ACT & ASSERT
        assert_eq!(PK_VARIABLE.name(), "PK");
        assert_eq!(PK_VARIABLE.vendor_guid(), &EFI_GLOBAL_VARIABLE);
        assert_eq!(DB_VARIABLE.name(), "db");
        assert_eq!(DB_VARIABLE.vendor_guid(), &EFI_IMAGE_SECURITY_DATABASE);
    }
}
