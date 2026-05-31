#[cfg(test)]
mod tests {
    use core::cell::RefCell;

    use sbolt::efi::enrollment;
    use sbolt::efi::siglist;
    use sbolt::efi::status;
    use sbolt::efi::variables::{self, Backend, Id, Update};
    use sbolt::error::Result;
    use sbolt::keys::hierarchy::{self, KeyType};
    use sbolt::keys::storage::{load_hierarchy, save_hierarchy};

    #[derive(Default)]
    struct FakeBackend {
        firmware_boot: bool,
        available: bool,
        ready: bool,
        variables: Vec<(Id, Vec<u8>)>,
        writes: RefCell<Vec<(Id, Vec<u8>)>>,
    }

    impl FakeBackend {
        fn ready() -> Self {
            Self {
                firmware_boot: true,
                available: true,
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

    impl Backend for FakeBackend {
        fn is_firmware_boot(&self) -> bool {
            self.firmware_boot
        }

        fn is_available(&self) -> bool {
            self.available
        }

        fn ensure_ready(&self) -> Result<bool> {
            Ok(self.ready)
        }

        fn variable_exists(&self, id: &Id) -> bool {
            self.variables
                .iter()
                .any(|&(stored_id, ref _payload)| stored_id == *id)
        }

        fn read_variable(&self, id: &Id) -> Result<Option<Vec<u8>>> {
            Ok(self
                .variables
                .iter()
                .find(|&&(stored_id, ref _payload)| stored_id == *id)
                .map(|&(_stored_id, ref payload)| payload.clone()))
        }

        fn write_variable(&self, update: Update<'_>) -> Result<()> {
            self.writes
                .borrow_mut()
                .push((*update.id(), update.payload().to_vec()));

            Ok(())
        }
    }

    #[test]
    fn public_efi_status_reads_from_fake_backend() {
        // ARRANGE
        let backend = FakeBackend::ready()
            .with_variable(variables::SECURE_BOOT, vec![1])
            .with_variable(variables::SETUP_MODE, vec![0])
            .with_variable(variables::PK, vec![1]);

        // ACT
        let booted = status::is_boot(&backend);
        let available = status::is_available(&backend);
        let ready = status::mount(&backend).expect("mount fake backend");
        let secure_boot = status::secure_boot(&backend).expect("read SecureBoot");
        let setup_mode = status::setup_mode(&backend).expect("read SetupMode");

        // ASSERT
        assert!(booted);
        assert!(available);
        assert!(ready);
        assert!(secure_boot);
        assert!(!setup_mode);
    }

    #[test]
    fn public_efi_status_reads_signature_databases_from_fake_backend() {
        // ARRANGE
        let owner = uefi::guid!("12345678-1234-1234-1234-123456789abc");
        let siglist = siglist::build_x509(&owner, b"cert-bytes").expect("build siglist");
        let backend = FakeBackend::ready()
            .with_variable(variables::PK, siglist.clone())
            .with_variable(variables::KEK, siglist.clone())
            .with_variable(variables::DB, siglist);

        // ACT
        let pk = status::pk(&backend).expect("read PK");
        let kek = status::kek(&backend).expect("read KEK");
        let db = status::db(&backend).expect("read db");

        // ASSERT
        assert_eq!(pk.map_or(0, |database| database.len()), 1);
        assert_eq!(kek.map_or(0, |database| database.len()), 1);
        assert_eq!(db.map_or(0, |database| database.len()), 1);
    }

    #[test]
    fn public_efi_enrollment_writes_expected_variables() {
        // ARRANGE
        let backend = FakeBackend::ready().with_variable(variables::SETUP_MODE, vec![1]);
        let hierarchy = hierarchy::Bundle::generate("Integration").expect("generate hierarchy");

        // ACT
        enrollment::enroll(&backend, &hierarchy).expect("enroll hierarchy");
        let writes = backend.writes();

        // ASSERT
        assert_eq!(writes.len(), 3);
        assert_eq!(
            writes.first().map(|&(id, ref _payload)| id),
            Some(variables::DB)
        );
        assert_eq!(
            writes.get(1).map(|&(id, ref _payload)| id),
            Some(variables::KEK)
        );
        assert_eq!(
            writes.get(2).map(|&(id, ref _payload)| id),
            Some(variables::PK)
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
        assert_eq!(loaded.owner_guid, hierarchy.owner_guid);
        assert_eq!(loaded.pk.key_type, KeyType::Pk);
        assert_eq!(loaded.kek.key_type, KeyType::Kek);
        assert_eq!(loaded.db.key_type, KeyType::Db);
    }
}
