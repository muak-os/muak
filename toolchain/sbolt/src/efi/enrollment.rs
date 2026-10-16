//! Platform-independent Secure Boot firmware variable enrollment.

use der::Encode as _;
use firmware::nvram::{Id, Update, VariableStore};
use firmware::secureboot;
use uefi::runtime::VariableAttributes;
use x509_cert::Certificate;

use super::authvar;
use super::siglist;
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
        secureboot::DB,
        &db_sigdb.to_bytes(),
        hierarchy,
        SigningAuthority::KeyExchangeKey,
    )
    .map_err(|e| SboltError::EfiVar(format!("failed to enroll db: {e}")))?;
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
        secureboot::PK,
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
fn write_signed_variable<S: VariableStore>(
    store: &S,
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

    Ok(store.write(Update::new(id, &payload))?)
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
    use core::cell::RefCell;

    use firmware::secureboot::{DB, KEK, PK, SETUP_MODE};

    use super::*;

    #[derive(Default)]
    struct FakeFirmwareVariablesStore {
        ready: bool,
        variables: Vec<(Id, Vec<u8>)>,
        writes: RefCell<Vec<(Id, Vec<u8>)>>,
    }

    impl FakeFirmwareVariablesStore {
        fn ready() -> Self {
            Self {
                ready: true,
                variables: Vec::new(),
                writes: RefCell::new(Vec::new()),
            }
        }

        fn with_variable(mut self, id: Id, payload: Vec<u8>) -> Self {
            self.variables.push((id, payload));
            self
        }

        fn writes(&self) -> Vec<(Id, Vec<u8>)> {
            self.writes.borrow().clone()
        }
    }

    impl VariableStore for FakeFirmwareVariablesStore {
        fn is_boot(&self) -> bool {
            true
        }

        fn is_available(&self) -> bool {
            true
        }

        fn prepare(&self) -> firmware::error::Result<bool> {
            Ok(self.ready)
        }

        fn exists(&self, id: &Id) -> bool {
            self.variables
                .iter()
                .any(|&(stored_id, ref _payload)| stored_id == *id)
        }

        fn read(&self, id: &Id) -> firmware::error::Result<Option<Vec<u8>>> {
            Ok(self
                .variables
                .iter()
                .find(|&&(stored_id, ref _payload)| stored_id == *id)
                .map(|&(_stored_id, ref payload)| payload.clone()))
        }

        fn write(&self, update: Update<'_>) -> firmware::error::Result<()> {
            self.writes
                .borrow_mut()
                .push((*update.id(), update.payload().to_vec()));

            Ok(())
        }
    }

    #[test]
    fn enroll_keys_writes_db_kek_and_pk_in_order() {
        // ARRANGE
        let store = FakeFirmwareVariablesStore::ready().with_variable(SETUP_MODE, vec![1]);
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
        enroll(&store, &hierarchy).expect("enroll hierarchy");
        let writes = store.writes();

        // ASSERT
        assert_eq!(writes.len(), 3);
        assert_eq!(writes.first().map(|&(id, ref _payload)| id), Some(DB));
        assert_eq!(writes.get(1).map(|&(id, ref _payload)| id), Some(KEK));
        assert_eq!(writes.get(2).map(|&(id, ref _payload)| id), Some(PK));
        assert!(
            writes
                .first()
                .is_some_and(|&(_id, ref payload)| payload.ends_with(&db_sigdb.to_bytes()))
        );
        assert!(
            writes
                .get(1)
                .is_some_and(|&(_id, ref payload)| payload.ends_with(&kek_sigdb.to_bytes()))
        );
        assert!(
            writes
                .get(2)
                .is_some_and(|&(_id, ref payload)| payload.ends_with(&pk_sigdb.to_bytes()))
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
        assert_ne!(database.to_bytes(), b"");
    }

    #[test]
    fn write_signed_variable_writes_payload_for_each_authority() {
        // ARRANGE
        let store = FakeFirmwareVariablesStore::ready();
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
    fn fake_variable_store_reports_status_and_variable_presence() {
        // ARRANGE
        let store = FakeFirmwareVariablesStore::ready().with_variable(PK, Vec::new());

        // ACT
        let firmware_boot = store.is_boot();
        let available = store.is_available();
        let has_pk = store.exists(&PK);
        let has_db = store.exists(&DB);

        // ASSERT
        assert!(firmware_boot);
        assert!(available);
        assert!(has_pk);
        assert!(!has_db);
    }

    #[test]
    fn enroll_keys_rejects_unavailable_variable_store() {
        // ARRANGE
        let store = FakeFirmwareVariablesStore {
            ready: false,
            ..FakeFirmwareVariablesStore::ready()
        };
        let hierarchy =
            hierarchy::Bundle::generate("Enroll Unavailable").expect("generate hierarchy");

        // ACT
        let result = enroll(&store, &hierarchy);

        // ASSERT
        result.expect_err("unavailable variable store should fail");
        assert_eq!(store.writes(), Vec::<(Id, alloc::vec::Vec<u8>)>::new());
    }

    #[test]
    fn enroll_keys_rejects_non_setup_mode() {
        // ARRANGE
        let store = FakeFirmwareVariablesStore::ready()
            .with_variable(SETUP_MODE, vec![0])
            .with_variable(PK, vec![1]);
        let hierarchy = hierarchy::Bundle::generate("Enroll No Setup").expect("generate hierarchy");

        // ACT
        let result = enroll(&store, &hierarchy);

        // ASSERT
        result.expect_err("non-setup mode should fail");
        assert_eq!(store.writes(), Vec::<(Id, alloc::vec::Vec<u8>)>::new());
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
