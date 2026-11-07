mod common;

#[cfg(test)]
mod tests {
    use der::Encode as _;
    use firmware::nvram::{Id, VariableStore as _};
    use firmware::secureboot::{self, DB, KEK, PK, SECURE_BOOT, SETUP_MODE};
    use sbolt::efi::{enrollment, siglist};
    use sbolt::keys::hierarchy::{self, KeyType};
    use sbolt::keys::storage::{load_hierarchy, save_hierarchy};
    use x509_cert::Certificate;

    use crate::common::Firmware;

    fn contains_bytes(payload: &[u8], needle: &[u8]) -> bool {
        payload.windows(needle.len()).any(|window| window == needle)
    }

    fn signed_database(owner_guid: &uefi::Guid, certificate: &Certificate) -> Vec<u8> {
        let cert_der = certificate.to_der().expect("encode certificate");
        let mut database = siglist::SignatureDatabase::new();
        database
            .add_x509(owner_guid, &cert_der)
            .expect("add certificate to database");

        database.to_bytes()
    }

    #[test]
    fn firmware_status_reads_from_fake_backend() {
        // ARRANGE
        let backend = Firmware::ready()
            .with_variable(SECURE_BOOT, vec![1])
            .with_variable(SETUP_MODE, vec![0])
            .with_variable(PK, vec![1]);

        // ACT
        let booted = backend.is_boot();
        let available = backend.is_available();
        let ready = backend.prepare().expect("mount fake backend");
        let secure_boot = secureboot::enabled(&backend).expect("read SecureBoot");
        let setup_mode = secureboot::setup_mode(&backend).expect("read SetupMode");

        // ASSERT
        assert!(booted, "fake backend must report EFI boot");
        assert!(available, "fake backend must report availability");
        assert!(ready, "fake backend must report readiness");
        assert!(secure_boot, "SecureBoot=1 must report enabled");
        assert!(!setup_mode, "SetupMode=0 must report disabled");
    }

    #[test]
    fn firmware_reports_status_and_variable_presence() {
        // ARRANGE
        let store = Firmware::ready().with_variable(PK, Vec::new());

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
    fn enroll_keys_writes_db_kek_and_pk_in_order() {
        // ARRANGE
        let store = Firmware::ready().with_variable(SETUP_MODE, vec![1]);
        let hierarchy = hierarchy::Bundle::generate("Enroll Success").expect("generate hierarchy");
        let db_sigdb = signed_database(&hierarchy.owner_guid, &hierarchy.db.certificate);
        let kek_sigdb = signed_database(&hierarchy.owner_guid, &hierarchy.kek.certificate);
        let pk_sigdb = signed_database(&hierarchy.owner_guid, &hierarchy.pk.certificate);

        // ACT
        enrollment::enroll(&store, &hierarchy).expect("enroll hierarchy");
        let writes = store.writes();

        // ASSERT
        assert_eq!(writes.len(), 3);
        assert_eq!(writes.first().map(|&(id, ref _payload)| id), Some(DB));
        assert_eq!(writes.get(1).map(|&(id, ref _payload)| id), Some(KEK));
        assert_eq!(writes.get(2).map(|&(id, ref _payload)| id), Some(PK));
        assert!(
            writes
                .first()
                .is_some_and(|&(_id, ref payload)| payload.ends_with(&db_sigdb))
        );
        assert!(
            writes
                .get(1)
                .is_some_and(|&(_id, ref payload)| payload.ends_with(&kek_sigdb))
        );
        assert!(
            writes
                .get(2)
                .is_some_and(|&(_id, ref payload)| payload.ends_with(&pk_sigdb))
        );
    }

    #[test]
    fn enroll_leaves_platform_in_setup_mode_when_a_write_fails_before_pk() {
        // ARRANGE
        let store = Firmware::ready()
            .with_variable(SETUP_MODE, vec![1])
            .failing_on(KEK);
        let hierarchy =
            hierarchy::Bundle::generate("Enroll Failure Before PK").expect("generate hierarchy");

        // ACT
        let result = enrollment::enroll(&store, &hierarchy);

        // ASSERT
        let error = result.expect_err("a failed KEK write must abort enrollment");
        assert!(
            error.to_string().contains("failed to enroll KEK"),
            "error must name the failing variable: {error}"
        );
        assert!(
            !store.exists(&PK),
            "PK must never be written when a prior write fails"
        );
        assert!(
            secureboot::setup_mode(&store).expect("read SetupMode"),
            "platform must remain in Setup Mode after a failure before PK"
        );

        // ARRANGE
        let healthy = Firmware::ready().with_variable(SETUP_MODE, vec![1]);

        // ACT
        let retry = enrollment::enroll(&healthy, &hierarchy);

        // ASSERT
        retry.expect("a retry on a healthy store must succeed");
    }

    #[test]
    fn enroll_rejects_firmware_that_does_not_persist_the_database() {
        // ARRANGE
        let store = Firmware::ready()
            .with_variable(SETUP_MODE, vec![1])
            .dropping_writes();
        let hierarchy =
            hierarchy::Bundle::generate("Enroll Missing Database").expect("generate hierarchy");

        // ACT
        let result = enrollment::enroll(&store, &hierarchy);

        // ASSERT
        result.expect_err("missing db after enrollment must fail verification");
        assert!(
            !store.writes().iter().any(|&(id, ref _payload)| id == PK),
            "PK must never be written when the db write is not persisted"
        );
    }

    #[test]
    fn enroll_does_not_write_pk_when_kek_is_not_persisted() {
        // ARRANGE
        let store = Firmware::ready()
            .with_variable(SETUP_MODE, vec![1])
            .ignoring_writes_on(KEK);
        let hierarchy =
            hierarchy::Bundle::generate("Enroll Dropped Kek").expect("generate hierarchy");

        // ACT
        let result = enrollment::enroll(&store, &hierarchy);

        // ASSERT
        let error = result.expect_err("a KEK that is not persisted must abort enrollment");
        assert!(
            error.to_string().contains("KEK"),
            "error must name the KEK: {error}"
        );
        assert!(
            !store.writes().iter().any(|&(id, ref _payload)| id == PK),
            "PK must never be written when the KEK write is not persisted"
        );
        assert!(
            secureboot::setup_mode(&store).expect("read SetupMode"),
            "platform must remain in Setup Mode when the KEK is not persisted"
        );
    }

    #[test]
    fn enroll_rejects_firmware_that_does_not_persist_the_platform_key() {
        // ARRANGE
        let store = Firmware::ready()
            .with_variable(SETUP_MODE, vec![1])
            .ignoring_writes_on(PK);
        let hierarchy =
            hierarchy::Bundle::generate("Enroll Dropped Pk").expect("generate hierarchy");

        // ACT
        let result = enrollment::enroll(&store, &hierarchy);

        // ASSERT
        let error = result.expect_err("a dropped PK write must not be reported as enrolled");
        assert!(
            error.to_string().contains("PK is absent"),
            "error must name the missing PK: {error}"
        );
        assert!(
            secureboot::setup_mode(&store).expect("read SetupMode"),
            "platform must remain in Setup Mode when the PK is not persisted"
        );
    }

    #[test]
    fn enroll_keys_rejects_unavailable_variable_store() {
        // ARRANGE
        let store = Firmware::ready().not_ready();
        let hierarchy =
            hierarchy::Bundle::generate("Enroll Unavailable").expect("generate hierarchy");

        // ACT
        let result = enrollment::enroll(&store, &hierarchy);

        // ASSERT
        result.expect_err("unavailable variable store should fail");
        assert_eq!(store.writes(), Vec::<(Id, Vec<u8>)>::new());
    }

    #[test]
    fn enroll_keys_rejects_non_setup_mode() {
        // ARRANGE
        let store = Firmware::ready()
            .with_variable(SETUP_MODE, vec![0])
            .with_variable(PK, vec![1]);
        let hierarchy = hierarchy::Bundle::generate("Enroll No Setup").expect("generate hierarchy");

        // ACT
        let result = enrollment::enroll(&store, &hierarchy);

        // ASSERT
        result.expect_err("non-setup mode should fail");
        assert_eq!(store.writes(), Vec::<(Id, Vec<u8>)>::new());
    }

    #[test]
    fn unenroll_removes_only_the_platform_key() {
        // ARRANGE
        let store = Firmware::ready()
            .with_variable(PK, vec![1])
            .with_variable(KEK, vec![1])
            .with_variable(DB, vec![1]);
        let hierarchy = hierarchy::Bundle::generate("Unenroll").expect("generate hierarchy");

        // ACT
        enrollment::unenroll(&store, &hierarchy).expect("unenroll hierarchy");
        let writes = store.writes();

        // ASSERT
        assert_eq!(writes.len(), 1, "only the Platform Key is deleted");
        assert_eq!(writes.first().map(|&(id, ref _payload)| id), Some(PK));
        assert!(!store.exists(&PK), "PK must be removed");
        assert!(store.exists(&KEK), "KEK must be left in place");
        assert!(store.exists(&DB), "db must be left in place");
    }

    #[test]
    fn clear_removes_the_whole_hierarchy() {
        // ARRANGE
        let store = Firmware::ready()
            .with_variable(PK, vec![1])
            .with_variable(KEK, vec![1])
            .with_variable(DB, vec![1]);
        let hierarchy = hierarchy::Bundle::generate("Clear").expect("generate hierarchy");
        let kek_der = hierarchy
            .kek
            .certificate
            .to_der()
            .expect("encode KEK certificate");
        let pk_der = hierarchy
            .pk
            .certificate
            .to_der()
            .expect("encode PK certificate");

        // ACT
        enrollment::clear(&store, &hierarchy).expect("clear hierarchy");
        let writes = store.writes();

        // ASSERT
        assert_eq!(writes.len(), 3, "PK, db, and KEK are deleted");
        assert_eq!(writes.first().map(|&(id, ref _payload)| id), Some(PK));
        assert_eq!(writes.get(1).map(|&(id, ref _payload)| id), Some(DB));
        assert_eq!(writes.get(2).map(|&(id, ref _payload)| id), Some(KEK));
        assert!(
            writes
                .get(1)
                .is_some_and(|&(_id, ref payload)| contains_bytes(payload, &kek_der)),
            "the db delete must be signed by the KEK"
        );
        assert!(
            writes
                .get(2)
                .is_some_and(|&(_id, ref payload)| contains_bytes(payload, &pk_der)),
            "the KEK delete must be signed by the PK"
        );
        assert!(!store.exists(&PK), "PK must be removed");
        assert!(!store.exists(&KEK), "KEK must be cleared");
        assert!(!store.exists(&DB), "db must be cleared");
    }

    #[test]
    fn clear_propagates_a_database_deletion_failure() {
        // ARRANGE
        let store = Firmware::ready()
            .with_variable(PK, vec![1])
            .with_variable(KEK, vec![1])
            .with_variable(DB, vec![1])
            .failing_on(DB);
        let hierarchy =
            hierarchy::Bundle::generate("Clear db Failure").expect("generate hierarchy");

        // ACT
        let result = enrollment::clear(&store, &hierarchy);

        // ASSERT
        let error = result.expect_err("a failed db deletion must abort clear");
        assert!(
            error.to_string().contains("failed to remove db"),
            "error must name the db variable: {error}"
        );
        assert!(
            !store.exists(&PK),
            "PK must already be removed when the db deletion is attempted"
        );
        assert!(
            secureboot::setup_mode(&store).expect("read SetupMode"),
            "a failure after the PK removal must leave the platform in Setup Mode"
        );
    }

    #[test]
    fn clear_reports_a_database_that_survives_deletion() {
        // ARRANGE
        let store = Firmware::ready()
            .with_variable(PK, vec![1])
            .with_variable(KEK, vec![1])
            .with_variable(DB, vec![1])
            .ignoring_writes_on(DB);
        let hierarchy =
            hierarchy::Bundle::generate("Clear Stubborn db").expect("generate hierarchy");

        // ACT
        let result = enrollment::clear(&store, &hierarchy);

        // ASSERT
        let error = result.expect_err("a db that survives deletion must not be reported as clear");
        assert!(
            error
                .to_string()
                .contains("firmware still reports an enrolled db"),
            "error must report the surviving db, not a write failure: {error}"
        );
        assert!(
            store.exists(&DB),
            "the accepted but unpersisted db deletion must leave the db enrolled"
        );
    }

    #[test]
    fn clear_reports_a_key_exchange_key_that_survives_deletion() {
        // ARRANGE
        let store = Firmware::ready()
            .with_variable(PK, vec![1])
            .with_variable(KEK, vec![1])
            .with_variable(DB, vec![1])
            .ignoring_writes_on(KEK);
        let hierarchy =
            hierarchy::Bundle::generate("Clear Stubborn KEK").expect("generate hierarchy");

        // ACT
        let result = enrollment::clear(&store, &hierarchy);

        // ASSERT
        let error = result.expect_err("a KEK that survives deletion must not be reported as clear");
        assert!(
            error
                .to_string()
                .contains("firmware still reports an enrolled KEK"),
            "error must report the surviving KEK, not a write failure: {error}"
        );
        assert!(
            store.exists(&KEK),
            "the accepted but unpersisted KEK deletion must leave the KEK enrolled"
        );
    }

    #[test]
    fn unenroll_is_idempotent_when_keys_are_absent() {
        // ARRANGE
        let store = Firmware::ready();
        let hierarchy = hierarchy::Bundle::generate("Unenroll Absent").expect("generate hierarchy");

        // ACT
        let result = enrollment::unenroll(&store, &hierarchy);

        // ASSERT
        result.expect("unenrolling an unenrolled store must succeed");
        assert!(
            store.writes().is_empty(),
            "no authenticated write is needed when nothing is enrolled"
        );
    }

    #[test]
    fn unenroll_rejects_firmware_that_keeps_the_platform_key() {
        // ARRANGE
        let store = Firmware::ready()
            .with_variable(PK, vec![1])
            .keeping_deletes();
        let hierarchy =
            hierarchy::Bundle::generate("Unenroll Stubborn").expect("generate hierarchy");

        // ACT
        let result = enrollment::unenroll(&store, &hierarchy);

        // ASSERT
        result.expect_err("firmware that keeps the Platform Key must fail verification");
    }

    #[test]
    fn unenroll_rejects_unavailable_variable_store() {
        // ARRANGE
        let store = Firmware::ready().not_ready();
        let hierarchy =
            hierarchy::Bundle::generate("Unenroll Unavailable").expect("generate hierarchy");

        // ACT
        let result = enrollment::unenroll(&store, &hierarchy);

        // ASSERT
        result.expect_err("unavailable variable store should fail");
    }

    #[test]
    fn clear_removes_signature_databases() {
        // ARRANGE
        let backend = Firmware::ready()
            .with_variable(DB, vec![1])
            .with_variable(KEK, vec![1]);
        let hierarchy =
            hierarchy::Bundle::generate("Clear Integration").expect("generate hierarchy");

        // ACT
        enrollment::clear(&backend, &hierarchy).expect("clear hierarchy");
        let writes = backend.writes();

        // ASSERT
        assert_eq!(
            writes
                .iter()
                .map(|&(id, ref _payload)| id)
                .collect::<Vec<_>>(),
            vec![DB, KEK],
            "db and KEK must be deleted"
        );
    }

    #[test]
    fn public_key_storage_api_round_trips_hierarchy() {
        // ARRANGE
        let hierarchy =
            hierarchy::Bundle::generate("Storage Integration").expect("generate hierarchy");
        let temp_dir = tempfile::tempdir().expect("create temp dir");

        // ACT
        save_hierarchy(&hierarchy, temp_dir.path()).expect("save hierarchy");
        let loaded = load_hierarchy(temp_dir.path()).expect("load hierarchy");

        // ASSERT
        assert_eq!(
            loaded.owner_guid, hierarchy.owner_guid,
            "owner GUID must round trip"
        );
        assert_eq!(loaded.pk.key_type, KeyType::Pk, "PK key type must survive");
        assert_eq!(
            loaded.kek.key_type,
            KeyType::Kek,
            "KEK key type must survive"
        );
        assert_eq!(loaded.db.key_type, KeyType::Db, "db key type must survive");
    }
}
