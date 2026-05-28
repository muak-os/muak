//! Platform-independent Secure Boot firmware variable enrollment.

use der::Encode as _;
use uefi::runtime::VariableAttributes;
use x509_cert::Certificate;

use super::authvar;
use super::siglist;
use super::status;
use super::variables::{self, Backend, Id, Update};
use crate::error::{Result, SboltError};
use crate::keys::hierarchy;
use crate::keys::rsa2048;

/// Signature authority for an authenticated firmware variable update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SigningAuthority {
    PlatformKey,
    KeyExchangeKey,
}

/// Enroll the complete key hierarchy into UEFI firmware.
///
/// # Errors
///
/// Returns an error if firmware variable storage is unavailable, the system is
/// not in Setup Mode, certificate encoding fails, or any variable write fails.
pub fn enroll<B: Backend>(backend: &B, hierarchy: &hierarchy::Bundle) -> Result<()> {
    if !backend.ensure_ready()? {
        return Err(SboltError::EfiVar("efivarfs not available".into()));
    }

    if !status::setup_mode(backend)? {
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
        variables::DB,
        &db_sigdb.to_bytes(),
        hierarchy,
        SigningAuthority::KeyExchangeKey,
    )
    .map_err(|e| SboltError::EfiVar(format!("failed to enroll db: {e}")))?;
    write_signed_variable(
        backend,
        variables::KEK,
        &kek_sigdb.to_bytes(),
        hierarchy,
        SigningAuthority::PlatformKey,
    )
    .map_err(|e| SboltError::EfiVar(format!("failed to enroll KEK: {e}")))?;
    write_signed_variable(
        backend,
        variables::PK,
        &pk_sigdb.to_bytes(),
        hierarchy,
        SigningAuthority::PlatformKey,
    )
    .map_err(|e| SboltError::EfiVar(format!("failed to enroll PK: {e}")))?;

    Ok(())
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
fn write_signed_variable<B: Backend>(
    backend: &B,
    id: Id,
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

    backend.write_variable(Update::new(id, &payload))
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

    use super::*;

    /// In-memory firmware variable backend for enrollment tests.
    #[derive(Default)]
    struct FakeFirmwareVariables {
        ready: bool,
        variables: Vec<(Id, Vec<u8>)>,
        writes: RefCell<Vec<(Id, Vec<u8>)>>,
    }

    impl FakeFirmwareVariables {
        /// Create a ready fake firmware variable backend.
        fn ready() -> Self {
            Self {
                ready: true,
                variables: Vec::new(),
                writes: RefCell::new(Vec::new()),
            }
        }

        /// Add a firmware variable to the fake backend.
        fn with_variable(mut self, id: Id, payload: Vec<u8>) -> Self {
            self.variables.push((id, payload));
            self
        }

        /// Return recorded firmware variable writes.
        fn writes(&self) -> Vec<(Id, Vec<u8>)> {
            self.writes.borrow().clone()
        }
    }

    impl Backend for FakeFirmwareVariables {
        /// Return whether the fake system was EFI-booted.
        fn is_firmware_boot(&self) -> bool {
            true
        }

        /// Return whether the fake firmware variable store is available.
        fn is_available(&self) -> bool {
            true
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

        /// Record a fake firmware variable write.
        fn write_variable(&self, update: Update<'_>) -> Result<()> {
            self.writes
                .borrow_mut()
                .push((*update.id(), update.payload().to_vec()));

            Ok(())
        }
    }

    #[test]
    fn enroll_keys_writes_db_kek_and_pk_in_order() {
        // ARRANGE
        let backend = FakeFirmwareVariables::ready().with_variable(variables::SETUP_MODE, vec![1]);
        let hierarchy = hierarchy::Bundle::generate("Enroll Success").expect("generate hierarchy");
        let db_sigdb = certificate_database(
            &hierarchy.owner_guid,
            &hierarchy.db.certificate,
            "encode db cert",
        )
        .expect("build db sigdb");
        let kek_sigdb = certificate_database(
            &hierarchy.owner_guid,
            &hierarchy.kek.certificate,
            "encode kek cert",
        )
        .expect("build KEK sigdb");
        let pk_sigdb = certificate_database(
            &hierarchy.owner_guid,
            &hierarchy.pk.certificate,
            "encode pk cert",
        )
        .expect("build PK sigdb");

        // ACT
        enroll(&backend, &hierarchy).expect("enroll hierarchy");
        let writes = backend.writes();

        // ASSERT
        assert_eq!(writes.len(), 3);
        assert_eq!(
            writes.first().map(|(id, _payload)| *id),
            Some(variables::DB)
        );
        assert_eq!(
            writes.get(1).map(|(id, _payload)| *id),
            Some(variables::KEK)
        );
        assert_eq!(writes.get(2).map(|(id, _payload)| *id), Some(variables::PK));
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
    }

    #[test]
    fn certificate_database_builds_single_x509_list() {
        // ARRANGE
        let hierarchy =
            hierarchy::Bundle::generate("Certificate Database").expect("generate hierarchy");

        // ACT
        let database = certificate_database(
            &hierarchy.owner_guid,
            &hierarchy.db.certificate,
            "encode test cert",
        )
        .expect("build certificate database");

        // ASSERT
        assert_eq!(database.len(), 1);
        assert!(!database.to_bytes().is_empty());
    }

    #[test]
    fn write_signed_variable_writes_payload_for_each_authority() {
        // ARRANGE
        let backend = FakeFirmwareVariables::ready();
        let hierarchy =
            hierarchy::Bundle::generate("Write Signed Variable").expect("generate hierarchy");
        let content = b"content";

        // ACT
        write_signed_variable(
            &backend,
            variables::PK,
            content,
            &hierarchy,
            SigningAuthority::PlatformKey,
        )
        .expect("write PK signed variable");
        write_signed_variable(
            &backend,
            variables::DB,
            content,
            &hierarchy,
            SigningAuthority::KeyExchangeKey,
        )
        .expect("write db signed variable");
        let writes = backend.writes();

        // ASSERT
        assert_eq!(writes.len(), 2);
        assert_eq!(
            writes.first().map(|(id, _payload)| *id),
            Some(variables::PK)
        );
        assert_eq!(writes.get(1).map(|(id, _payload)| *id), Some(variables::DB));
        assert!(
            writes
                .iter()
                .all(|(_id, payload)| payload.ends_with(content))
        );
    }

    #[test]
    fn fake_backend_reports_status_and_variable_presence() {
        // ARRANGE
        let backend = FakeFirmwareVariables::ready().with_variable(variables::PK, Vec::new());

        // ACT
        let firmware_boot = backend.is_firmware_boot();
        let available = backend.is_available();
        let has_pk = backend.variable_exists(&variables::PK);
        let has_db = backend.variable_exists(&variables::DB);

        // ASSERT
        assert!(firmware_boot);
        assert!(available);
        assert!(has_pk);
        assert!(!has_db);
    }

    #[test]
    fn enroll_keys_rejects_unavailable_backend() {
        // ARRANGE
        let backend = FakeFirmwareVariables {
            ready: false,
            ..FakeFirmwareVariables::ready()
        };
        let hierarchy =
            hierarchy::Bundle::generate("Enroll Unavailable").expect("generate hierarchy");

        // ACT
        let result = enroll(&backend, &hierarchy);

        // ASSERT
        assert!(result.is_err());
        assert!(backend.writes().is_empty());
    }

    #[test]
    fn enroll_keys_rejects_non_setup_mode() {
        // ARRANGE
        let backend = FakeFirmwareVariables::ready()
            .with_variable(variables::SETUP_MODE, vec![0])
            .with_variable(variables::PK, vec![1]);
        let hierarchy = hierarchy::Bundle::generate("Enroll No Setup").expect("generate hierarchy");

        // ACT
        let result = enroll(&backend, &hierarchy);

        // ASSERT
        assert!(result.is_err());
        assert!(backend.writes().is_empty());
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
}
