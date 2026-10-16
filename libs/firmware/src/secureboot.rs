//! Secure Boot firmware state.

use crate::error::Result;
use crate::nvram::{EFI_GLOBAL_VARIABLE, EFI_IMAGE_SECURITY_DATABASE, Id, VariableStore};

/// Secure Boot enabled flag variable.
pub const SECURE_BOOT: Id = Id::new("SecureBoot", EFI_GLOBAL_VARIABLE);

/// Setup Mode flag variable.
pub const SETUP_MODE: Id = Id::new("SetupMode", EFI_GLOBAL_VARIABLE);

/// Platform Key variable.
pub const PK: Id = Id::new("PK", EFI_GLOBAL_VARIABLE);

/// Key Exchange Key variable.
pub const KEK: Id = Id::new("KEK", EFI_GLOBAL_VARIABLE);

/// Signature database variable.
pub const DB: Id = Id::new("db", EFI_IMAGE_SECURITY_DATABASE);

/// Snapshot of Secure Boot firmware state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct State {
    /// Secure Boot enforcement is enabled.
    pub enabled: bool,
    /// Firmware is in Setup Mode, meaning no Platform Key is enrolled.
    pub setup_mode: bool,
    /// A Platform Key is present.
    pub pk_present: bool,
}

/// Reads Secure Boot state from the active variable store.
///
/// # Errors
///
/// Returns an error if a firmware variable cannot be read.
pub fn system() -> Result<State> {
    state(&crate::system())
}

/// Reads Secure Boot state from `store`.
///
/// # Errors
///
/// Returns an error if a firmware variable cannot be read.
pub fn state<S: VariableStore>(store: &S) -> Result<State> {
    Ok(State {
        enabled: enabled(store)?,
        setup_mode: setup_mode(store)?,
        pk_present: pk_present(store)?,
    })
}

/// Returns whether Secure Boot is enabled.
///
/// # Errors
///
/// Returns an error if the Secure Boot variable cannot be read.
pub fn enabled<S: VariableStore>(store: &S) -> Result<bool> {
    boolean(store, &SECURE_BOOT)
}

/// Returns whether the system is in Setup Mode.
///
/// # Errors
///
/// Returns an error if the Setup Mode variable cannot be read.
pub fn setup_mode<S: VariableStore>(store: &S) -> Result<bool> {
    if let Some(data) = store.read(&SETUP_MODE)?
        && !data.is_empty()
    {
        return Ok(data.first().copied() == Some(1));
    }

    Ok(!store.exists(&PK))
}

/// Returns whether a Platform Key is present.
///
/// # Errors
///
/// Returns an error if the Platform Key variable cannot be read.
pub fn pk_present<S: VariableStore>(store: &S) -> Result<bool> {
    Ok(store.read(&PK)?.is_some_and(|data| !data.is_empty()))
}

/// Reads a boolean firmware variable.
fn boolean<S: VariableStore>(store: &S, id: &Id) -> Result<bool> {
    match store.read(id)? {
        Some(data) if !data.is_empty() => Ok(data.first().copied() == Some(1)),
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nvram::Update;

    #[derive(Default)]
    struct Fake {
        ready: bool,
        variables: Vec<(Id, Vec<u8>)>,
    }

    impl Fake {
        fn ready() -> Self {
            Self {
                ready: true,
                variables: Vec::new(),
            }
        }

        fn with(mut self, id: Id, payload: Vec<u8>) -> Self {
            self.variables.push((id, payload));
            self
        }
    }

    impl VariableStore for Fake {
        fn is_boot(&self) -> bool {
            true
        }

        fn is_available(&self) -> bool {
            true
        }

        fn prepare(&self) -> Result<bool> {
            Ok(self.ready)
        }

        fn exists(&self, id: &Id) -> bool {
            self.variables.iter().any(|entry| entry.0 == *id)
        }

        fn read(&self, id: &Id) -> Result<Option<Vec<u8>>> {
            Ok(self
                .variables
                .iter()
                .find(|entry| entry.0 == *id)
                .map(|entry| entry.1.clone()))
        }

        fn write(&self, _update: Update<'_>) -> Result<()> {
            Ok(())
        }

        fn remove(&self, _id: &Id) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn state_follows_variable_contents() {
        // ARRANGE
        let store = Fake::ready()
            .with(SECURE_BOOT, vec![1])
            .with(SETUP_MODE, vec![0])
            .with(PK, vec![1]);

        // ACT
        let state = state(&store).expect("read state");

        // ASSERT
        assert!(state.enabled, "SecureBoot=1 must report enabled");
        assert!(!state.setup_mode, "SetupMode=0 must report disabled");
        assert!(state.pk_present, "non-empty PK must report present");
    }

    #[test]
    fn enabled_is_false_for_missing_and_empty_variables() {
        // ARRANGE
        let missing = Fake::ready();
        let empty = Fake::ready().with(SECURE_BOOT, Vec::new());

        // ACT
        let missing_enabled = enabled(&missing).expect("read missing");
        let empty_enabled = enabled(&empty).expect("read empty");

        // ASSERT
        assert!(!missing_enabled, "missing SecureBoot must be disabled");
        assert!(!empty_enabled, "empty SecureBoot must be disabled");
    }

    #[test]
    fn setup_mode_falls_back_to_pk_presence() {
        // ARRANGE
        let with_pk = Fake::ready().with(PK, vec![1]);
        let without_pk = Fake::ready();

        // ACT
        let setup_with_pk = setup_mode(&with_pk).expect("read with PK");
        let setup_without_pk = setup_mode(&without_pk).expect("read without PK");

        // ASSERT
        assert!(!setup_with_pk, "a present PK means Setup Mode is off");
        assert!(setup_without_pk, "a missing PK means Setup Mode is on");
    }

    #[test]
    fn pk_present_ignores_empty_payloads() {
        // ARRANGE
        let empty = Fake::ready().with(PK, Vec::new());
        let filled = Fake::ready().with(PK, vec![1]);

        // ACT
        let empty_present = pk_present(&empty).expect("read empty");
        let filled_present = pk_present(&filled).expect("read filled");

        // ASSERT
        assert!(!empty_present, "empty PK must not count as present");
        assert!(filled_present, "non-empty PK must count as present");
    }

    #[test]
    fn variable_ids_use_expected_namespaces() {
        // ACT & ASSERT
        assert_eq!(PK.name(), "PK");
        assert_eq!(PK.vendor_guid(), &EFI_GLOBAL_VARIABLE);
        assert_eq!(DB.name(), "db");
        assert_eq!(DB.vendor_guid(), &EFI_IMAGE_SECURITY_DATABASE);
    }
}
