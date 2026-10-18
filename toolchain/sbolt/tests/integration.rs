#[cfg(test)]
mod tests {
    use core::cell::RefCell;

    use firmware::error::Result as FwResult;
    use firmware::nvram::{Id, Update, VariableStore};
    use firmware::secureboot::{self, DB, KEK, PK, SECURE_BOOT, SETUP_MODE};
    use sbolt::efi::enrollment;
    use sbolt::keys::hierarchy::{self, KeyType};
    use sbolt::keys::storage::{load_hierarchy, save_hierarchy};

    #[derive(Default)]
    struct FakeBackend {
        firmware_boot: bool,
        available: bool,
        ready: bool,
        variables: RefCell<Vec<(Id, Vec<u8>)>>,
        writes: RefCell<Vec<(Id, Vec<u8>)>>,
    }

    impl FakeBackend {
        fn ready() -> Self {
            Self {
                firmware_boot: true,
                available: true,
                ready: true,
                variables: RefCell::new(Vec::new()),
                writes: RefCell::new(Vec::new()),
            }
        }

        fn with_variable(self, id: Id, payload: Vec<u8>) -> Self {
            self.variables.borrow_mut().push((id, payload));
            self
        }

        fn writes(&self) -> Vec<(Id, Vec<u8>)> {
            self.writes.borrow().clone()
        }
    }

    impl VariableStore for FakeBackend {
        fn is_boot(&self) -> bool {
            self.firmware_boot
        }

        fn is_available(&self) -> bool {
            self.available
        }

        fn prepare(&self) -> FwResult<bool> {
            Ok(self.ready)
        }

        fn exists(&self, id: &Id) -> bool {
            self.variables.borrow().iter().any(|entry| entry.0 == *id)
        }

        fn read(&self, id: &Id) -> FwResult<Option<Vec<u8>>> {
            Ok(self
                .variables
                .borrow()
                .iter()
                .find(|entry| entry.0 == *id)
                .map(|entry| entry.1.clone()))
        }

        fn write(&self, update: Update<'_>) -> FwResult<()> {
            let id = *update.id();
            self.writes
                .borrow_mut()
                .push((id, update.payload().to_vec()));
            persist_variable(&self.variables, id, update.payload());

            Ok(())
        }
    }

    fn persist_variable(variables: &RefCell<Vec<(Id, Vec<u8>)>>, id: Id, payload: &[u8]) {
        let content = signed_content(payload).to_vec();
        let mut variables = variables.borrow_mut();
        variables.retain(|entry| entry.0 != id);
        if !content.is_empty() {
            variables.push((id, content));
        }
    }

    fn signed_content(payload: &[u8]) -> &[u8] {
        let certificate_size = payload
            .get(20..24)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes)
            .and_then(|size| usize::try_from(size).ok())
            .unwrap_or(0);
        let content_offset = 20_usize.saturating_add(certificate_size);

        payload.get(content_offset..).unwrap_or_default()
    }

    #[test]
    fn firmware_status_reads_from_fake_backend() {
        // ARRANGE
        let backend = FakeBackend::ready()
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
    fn enrollment_writes_expected_variables() {
        // ARRANGE
        let backend = FakeBackend::ready().with_variable(SETUP_MODE, vec![1]);
        let hierarchy = hierarchy::Bundle::generate("Integration").expect("generate hierarchy");

        // ACT
        enrollment::enroll(&backend, &hierarchy).expect("enroll hierarchy");
        let writes = backend.writes();

        // ASSERT
        assert_eq!(writes.len(), 3, "PK, KEK, and db must be written");
        assert_eq!(
            writes.first().map(|&(id, ref _payload)| id),
            Some(PK),
            "PK must be written first"
        );
        assert_eq!(
            writes.get(1).map(|&(id, ref _payload)| id),
            Some(KEK),
            "KEK must be written second"
        );
        assert_eq!(
            writes.get(2).map(|&(id, ref _payload)| id),
            Some(DB),
            "db must be written last"
        );
    }

    #[test]
    fn clear_removes_signature_databases() {
        // ARRANGE
        let backend = FakeBackend::ready()
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
