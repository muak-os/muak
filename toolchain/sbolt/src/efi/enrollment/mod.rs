//! Platform-independent Secure Boot firmware variable enrollment and removal.

mod checks;
mod removal;

use checks::{verify_database, verify_platform_key_absent};
use der::Encode as _;
use firmware::nvram::{Id, Update, VariableStore};
use firmware::secureboot;
use removal::{clear_database, delete_signed_variable};
use uefi::runtime::VariableAttributes;
use x509_cert::Certificate;

use super::{authvar, siglist};
use crate::error::{Result, SboltError};
use crate::keys::{hierarchy, rsa2048};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SigningAuthority {
    PlatformKey,
    KeyExchangeKey,
}

/// Enroll the complete key hierarchy into UEFI firmware.
///
/// # Errors
///
/// Returns an error if firmware variable storage is unavailable, the system is
/// not in Setup Mode, certificate encoding fails, or any variable write fails.
pub fn enroll<S: VariableStore>(store: &S, hierarchy: &hierarchy::Bundle) -> Result<()> {
    if !store.prepare()? {
        return Err(SboltError::EfiVar("efivarfs not available".into()));
    }

    if !secureboot::setup_mode(store)? {
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
        store,
        secureboot::PK,
        &pk_sigdb.to_bytes(),
        hierarchy,
        SigningAuthority::PlatformKey,
    )
    .map_err(|e| SboltError::EfiVar(format!("failed to enroll PK: {e}")))?;
    write_signed_variable(
        store,
        secureboot::KEK,
        &kek_sigdb.to_bytes(),
        hierarchy,
        SigningAuthority::PlatformKey,
    )
    .map_err(|e| SboltError::EfiVar(format!("failed to enroll KEK: {e}")))?;
    write_signed_variable(
        store,
        secureboot::DB,
        &db_sigdb.to_bytes(),
        hierarchy,
        SigningAuthority::KeyExchangeKey,
    )
    .map_err(|e| SboltError::EfiVar(format!("failed to enroll db: {e}")))?;

    verify_database(store, hierarchy)
}

/// Remove the Platform Key and leave Secure Boot enforcement.
///
/// # Errors
///
/// Returns an error if firmware variable storage is unavailable, the Platform Key cannot
/// be deleted, or the firmware still reports an enrolled Platform Key after the removal.
pub fn unenroll<S: VariableStore>(store: &S, hierarchy: &hierarchy::Bundle) -> Result<()> {
    if !store.prepare()? {
        return Err(SboltError::EfiVar("efivarfs not available".into()));
    }

    delete_signed_variable(
        store,
        secureboot::PK,
        hierarchy,
        SigningAuthority::PlatformKey,
    )
    .map_err(|e| SboltError::EfiVar(format!("failed to remove PK: {e}")))?;

    verify_platform_key_absent(store)
}

/// Remove the complete key hierarchy from UEFI firmware.
///
/// # Errors
///
/// Returns an error if firmware variable storage is unavailable, the Platform Key cannot
/// be deleted, or the firmware still reports an enrolled Platform Key after the removal.
pub fn clear<S: VariableStore>(store: &S, hierarchy: &hierarchy::Bundle) -> Result<()> {
    unenroll(store, hierarchy)?;

    clear_database(
        store,
        secureboot::DB,
        hierarchy,
        SigningAuthority::KeyExchangeKey,
    );
    clear_database(
        store,
        secureboot::KEK,
        hierarchy,
        SigningAuthority::PlatformKey,
    );

    Ok(())
}

pub(super) fn write_signed_variable<S: VariableStore>(
    store: &S,
    id: Id,
    content: &[u8],
    hierarchy: &hierarchy::Bundle,
    authority: SigningAuthority,
) -> Result<()> {
    let (signer, certificate) = signer_and_certificate(hierarchy, authority);
    let attributes = authenticated_variable_attributes();
    let payload = authvar::sign(
        id.name(),
        id.vendor_guid(),
        attributes,
        content,
        signer,
        certificate,
    )?;

    Ok(store.write(Update::new(id, attributes.bits(), &payload))?)
}

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

fn signer_and_certificate(
    hierarchy: &hierarchy::Bundle,
    authority: SigningAuthority,
) -> (&rsa2048::Signer, &Certificate) {
    match authority {
        SigningAuthority::PlatformKey => (&hierarchy.pk.signer, &hierarchy.pk.certificate),
        SigningAuthority::KeyExchangeKey => (&hierarchy.kek.signer, &hierarchy.kek.certificate),
    }
}

fn authenticated_variable_attributes() -> VariableAttributes {
    VariableAttributes::NON_VOLATILE
        .union(VariableAttributes::BOOTSERVICE_ACCESS)
        .union(VariableAttributes::RUNTIME_ACCESS)
        .union(VariableAttributes::TIME_BASED_AUTHENTICATED_WRITE_ACCESS)
}

#[cfg(test)]
mod tests {
    use core::cell::RefCell;

    use firmware::secureboot::{DB, PK};

    use super::*;

    #[derive(Default)]
    struct RecordingWrites {
        writes: RefCell<Vec<(Id, Vec<u8>)>>,
    }

    impl RecordingWrites {
        fn writes(&self) -> Vec<(Id, Vec<u8>)> {
            self.writes.borrow().clone()
        }
    }

    impl VariableStore for RecordingWrites {
        fn is_boot(&self) -> bool {
            true
        }

        fn is_available(&self) -> bool {
            true
        }

        fn prepare(&self) -> firmware::error::Result<bool> {
            Ok(true)
        }

        fn exists(&self, _id: &Id) -> bool {
            false
        }

        fn read(&self, _id: &Id) -> firmware::error::Result<Option<Vec<u8>>> {
            Ok(None)
        }

        fn write(&self, update: Update<'_>) -> firmware::error::Result<()> {
            self.writes
                .borrow_mut()
                .push((*update.id(), update.data().to_vec()));

            Ok(())
        }
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
        assert_ne!(database.to_bytes(), b"");
    }

    #[test]
    fn write_signed_variable_writes_payload_for_each_authority() {
        // ARRANGE
        let store = RecordingWrites::default();
        let hierarchy =
            hierarchy::Bundle::generate("Write Signed Variable").expect("generate hierarchy");
        let content = b"content";

        // ACT
        write_signed_variable(
            &store,
            PK,
            content,
            &hierarchy,
            SigningAuthority::PlatformKey,
        )
        .expect("write PK signed variable");
        write_signed_variable(
            &store,
            DB,
            content,
            &hierarchy,
            SigningAuthority::KeyExchangeKey,
        )
        .expect("write db signed variable");
        let writes = store.writes();

        // ASSERT
        assert_eq!(writes.len(), 2);
        assert_eq!(writes.first().map(|&(id, ref _payload)| id), Some(PK));
        assert_eq!(writes.get(1).map(|&(id, ref _payload)| id), Some(DB));
        assert!(
            writes
                .iter()
                .all(|&(_id, ref payload)| payload.ends_with(content))
        );
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
