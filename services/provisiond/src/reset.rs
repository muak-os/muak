//! Factory reset functionality for removing STATE and DATA partitions.

use std::path::Path;

use ::disk::role::Role;
use anyhow::{Context as _, Result, bail};
use firmware::nvram::VariableStore;
use firmware::secureboot;
use sbolt::efi::enrollment;
use sbolt::keys::storage::load_hierarchy;

use crate::disk;
use crate::secrets::SECRETS_DIR;

/// Performs a factory reset by deleting the STATE and DATA partitions.
///
/// # Errors
///
/// Returns an error when the enrolled Secure Boot keys cannot be removed, when
/// their signing material is missing, or when a partition cannot be deleted.
pub fn factory_reset() -> Result<()> {
    kmsg::info!("Starting factory reset...");

    clear_secureboot(&firmware::system(), Path::new(SECRETS_DIR))?;

    disk::unmount_partition("/run/data")?;
    disk::unmount_partition("/run/state")?;

    if let Err(e) = luks2::close(Role::State.dm_name()) {
        kmsg::warn!("Failed to close LUKS STATE mapping (may not exist): {}", e);
    }
    if let Err(e) = luks2::close(Role::Data.dm_name()) {
        kmsg::warn!("Failed to close LUKS DATA mapping (may not exist): {}", e);
    }

    let state_device = disk::find_partition_device(Role::State)
        .context("STATE partition not found on any disk")?;
    let state_disk = disk::parent_disk(&state_device)
        .context("Failed to resolve the disk carrying the STATE partition")?;
    delete_role(&state_disk, Role::State)?;

    match disk::find_partition_device(Role::Data) {
        Some(data_device) => {
            let data_disk = disk::parent_disk(&data_device)
                .context("Failed to resolve the disk carrying the DATA partition")?;
            if data_disk == state_disk {
                delete_role(&state_disk, Role::Data)?;
            } else {
                delete_role(&data_disk, Role::Data)?;
            }
        }
        None => kmsg::info!("No DATA partition found, nothing to delete"),
    }

    kmsg::info!("Factory reset complete");

    Ok(())
}

fn clear_secureboot<S: VariableStore>(store: &S, secrets_dir: &Path) -> Result<()> {
    if !secureboot::pk_present(store)? {
        return Ok(());
    }

    let dir = secrets_dir.join("secureboot");
    if !dir.exists() {
        bail!(
            "Secure Boot keys are enrolled but the signing material on STATE is missing; \
             clear the keys from firmware setup before resetting"
        );
    }

    let hierarchy = load_hierarchy(&dir).context("Failed to load Secure Boot keys for removal")?;
    enrollment::clear(store, &hierarchy).context("Failed to remove Secure Boot keys")?;
    kmsg::info!("Secure Boot keys removed from firmware");

    Ok(())
}

fn delete_role(disk: &str, role: Role) -> Result<()> {
    let name = role.gpt_name();
    match disk::find_partition_number(disk, name)? {
        Some(number) => disk::delete_partitions(disk, &[number])?,
        None => kmsg::info!("No '{name}' partition found on {disk}, nothing to delete"),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use firmware::nvram::{Id, Update};
    use sbolt::keys::hierarchy::Bundle;
    use sbolt::keys::storage::save_hierarchy;

    use super::*;

    #[derive(Default)]
    struct FakeStore {
        variables: RefCell<Vec<(Id, Vec<u8>)>>,
        writes: RefCell<Vec<Id>>,
    }

    impl FakeStore {
        fn with(self, id: Id, payload: Vec<u8>) -> Self {
            self.variables.borrow_mut().push((id, payload));
            self
        }

        fn writes(&self) -> Vec<Id> {
            self.writes.borrow().clone()
        }
    }

    impl VariableStore for FakeStore {
        fn is_boot(&self) -> bool {
            true
        }

        fn is_available(&self) -> bool {
            true
        }

        fn prepare(&self) -> firmware::error::Result<bool> {
            Ok(true)
        }

        fn exists(&self, id: &Id) -> bool {
            self.variables.borrow().iter().any(|entry| entry.0 == *id)
        }

        fn read(&self, id: &Id) -> firmware::error::Result<Option<Vec<u8>>> {
            Ok(self
                .variables
                .borrow()
                .iter()
                .find(|entry| entry.0 == *id)
                .map(|entry| entry.1.clone()))
        }

        fn write(&self, update: Update<'_>) -> firmware::error::Result<()> {
            let id = *update.id();
            self.writes.borrow_mut().push(id);
            self.variables.borrow_mut().retain(|entry| entry.0 != id);

            Ok(())
        }
    }

    #[test]
    fn clear_secureboot_removes_enrolled_keys() {
        // ARRANGE
        let secrets_dir = tempfile::tempdir().expect("temp dir");
        let hierarchy = Bundle::generate("Reset").expect("generate hierarchy");
        save_hierarchy(&hierarchy, &secrets_dir.path().join("secureboot")).expect("save hierarchy");
        let store = FakeStore::default()
            .with(secureboot::PK, vec![1])
            .with(secureboot::KEK, vec![1])
            .with(secureboot::DB, vec![1]);

        // ACT
        clear_secureboot(&store, secrets_dir.path()).expect("clear Secure Boot keys");

        // ASSERT
        assert!(!store.exists(&secureboot::PK), "PK must be removed");
        assert!(!store.exists(&secureboot::KEK), "KEK must be removed");
        assert!(!store.exists(&secureboot::DB), "db must be removed");
    }

    #[test]
    fn clear_secureboot_rejects_missing_signing_material() {
        // ARRANGE
        let secrets_dir = tempfile::tempdir().expect("temp dir");
        let store = FakeStore::default().with(secureboot::PK, vec![1]);

        // ACT
        let result = clear_secureboot(&store, secrets_dir.path());

        // ASSERT
        result.expect_err("missing signing material must fail");
        assert!(store.writes().is_empty(), "no firmware write may happen");
    }

    #[test]
    fn clear_secureboot_is_noop_without_platform_key() {
        // ARRANGE
        let secrets_dir = tempfile::tempdir().expect("temp dir");
        let store = FakeStore::default();

        // ACT
        let result = clear_secureboot(&store, secrets_dir.path());

        // ASSERT
        result.expect("no enrolled key must succeed");
        assert!(store.writes().is_empty(), "no firmware write may happen");
    }
}
