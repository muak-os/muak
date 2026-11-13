//! Secure Boot firmware state.

use crate::error::Result;
use crate::nvram::{EFI_GLOBAL_VARIABLE, EFI_IMAGE_SECURITY_DATABASE, Id, VariableStore};
use crate::signature;

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

/// Secure Boot enforcement posture reported by firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Firmware is enforcing Secure Boot.
    Enabled,
    /// Our keys are enrolled but enforcement is not active yet.
    Pending,
    /// Our keys are not enrolled, so Muak does not own Secure Boot.
    Disabled,
}

/// Disagreement between the configured intent and the firmware state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mismatch {
    /// Configuration enables Secure Boot but our Platform Key is not enrolled.
    NotEnrolled,
    /// Configuration disables Secure Boot but our Platform Key is still enrolled.
    KeysRemain,
    /// Configuration disables Secure Boot but firmware still enforces it.
    EnforcingWhileDisabled,
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
/// Returns an error if the Setup Mode or Platform Key variable cannot be read.
pub fn setup_mode<S: VariableStore>(store: &S) -> Result<bool> {
    if let Some(data) = store.read(&SETUP_MODE)?
        && !data.is_empty()
    {
        return Ok(data.first().copied() == Some(1));
    }

    Ok(!present(store, &PK)?)
}

/// Returns whether a Platform Key is present.
///
/// # Errors
///
/// Returns an error if the Platform Key variable cannot be read.
pub fn pk_present<S: VariableStore>(store: &S) -> Result<bool> {
    present(store, &PK)
}

/// Returns whether the variable `id` is present with a non-empty payload.
///
/// # Errors
///
/// Returns an error if the variable cannot be read.
pub fn present<S: VariableStore>(store: &S, id: &Id) -> Result<bool> {
    Ok(store.read(id)?.is_some_and(|data| !data.is_empty()))
}

/// Returns whether the enrolled Platform Key contains `certificate`.
///
/// # Errors
///
/// Returns an error if the Platform Key variable cannot be read.
pub fn platform_key_matches<S: VariableStore>(store: &S, certificate: &[u8]) -> Result<bool> {
    let Some(data) = store.read(&PK)? else {
        return Ok(false);
    };

    Ok(signature::contains_x509(&data, certificate))
}

/// Derives the Secure Boot posture, treating only our Platform Key as enrolled.
#[must_use = "callers must branch on the Secure Boot posture"]
pub fn status(state: &State, ours: bool) -> Status {
    if state.enabled {
        Status::Enabled
    } else if ours {
        Status::Pending
    } else {
        Status::Disabled
    }
}

/// Describes how config disagrees with our keys in firmware, if at all.
#[must_use = "callers must handle config disagreeing with the enrolled keys"]
pub fn mismatch(config_enabled: bool, state: &State, ours: bool) -> Option<Mismatch> {
    match (config_enabled, state.enabled, ours) {
        (true, _, false) => Some(Mismatch::NotEnrolled),
        (false, true, _) => Some(Mismatch::EnforcingWhileDisabled),
        (false, false, true) => Some(Mismatch::KeysRemain),
        _ => None,
    }
}

// Reads a boolean firmware variable.
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
    }

    fn x509_signature_list(certificate: &[u8]) -> Vec<u8> {
        const LIST_HEADER_SIZE: usize = 28;
        const DATA_HEADER_SIZE: usize = 16;
        let signature_size = DATA_HEADER_SIZE
            .checked_add(certificate.len())
            .expect("signature size");
        let list_size = LIST_HEADER_SIZE
            .checked_add(signature_size)
            .expect("list size");
        let mut list = Vec::with_capacity(list_size);
        list.extend_from_slice(&uefi::guid!("a5c059a1-94e4-4aa7-87b5-ab155c2bf072").to_bytes());
        list.extend_from_slice(&u32::try_from(list_size).expect("list size").to_le_bytes());
        list.extend_from_slice(&0_u32.to_le_bytes());
        list.extend_from_slice(
            &u32::try_from(signature_size)
                .expect("signature size")
                .to_le_bytes(),
        );
        list.extend_from_slice(&[0_u8; 16]);
        list.extend_from_slice(certificate);

        list
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
    fn setup_mode_prefers_an_explicit_variable_over_pk_presence() {
        // ARRANGE
        let store = Fake::ready().with(SETUP_MODE, vec![1]).with(PK, vec![1]);

        // ACT
        let setup_mode = setup_mode(&store).expect("read setup mode");

        // ASSERT
        assert!(
            setup_mode,
            "an explicit SetupMode=1 must win over a present PK"
        );
    }

    #[test]
    fn setup_mode_agrees_with_pk_presence_for_empty_platform_key() {
        // ARRANGE
        let store = Fake::ready().with(PK, Vec::new());

        // ACT
        let setup_mode = setup_mode(&store).expect("read setup mode");
        let pk_present = pk_present(&store).expect("read PK presence");

        // ASSERT
        assert!(setup_mode, "a zero-length PK must report Setup Mode");
        assert!(!pk_present, "a zero-length PK must not report present");
        assert_eq!(
            setup_mode, !pk_present,
            "Setup Mode must be the negation of PK presence"
        );
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

    #[test]
    fn status_follows_firmware_flags_and_key_ownership() {
        // ARRANGE
        let enforcing = State {
            enabled: true,
            setup_mode: false,
            pk_present: true,
        };
        let pending = State {
            enabled: false,
            setup_mode: false,
            pk_present: true,
        };
        let disabled = State {
            enabled: false,
            setup_mode: true,
            pk_present: false,
        };

        // ACT & ASSERT
        assert_eq!(status(&enforcing, true), Status::Enabled);
        assert_eq!(status(&enforcing, false), Status::Enabled);
        assert_eq!(status(&pending, true), Status::Pending);
        assert_eq!(status(&pending, false), Status::Disabled);
        assert_eq!(status(&disabled, false), Status::Disabled);
    }

    #[test]
    fn pending_requires_our_platform_key() {
        // ARRANGE
        let foreign_pk = State {
            enabled: false,
            setup_mode: false,
            pk_present: true,
        };

        // ACT
        let status = status(&foreign_pk, false);

        // ASSERT
        assert_eq!(status, Status::Disabled, "a foreign PK must not be pending");
    }

    #[test]
    fn mismatch_reports_config_and_firmware_disagreement() {
        // ARRANGE
        let enforcing = State {
            enabled: true,
            setup_mode: false,
            pk_present: true,
        };
        let pending = State {
            enabled: false,
            setup_mode: false,
            pk_present: true,
        };
        let disabled = State {
            enabled: false,
            setup_mode: true,
            pk_present: false,
        };

        // ACT & ASSERT
        assert_eq!(mismatch(true, &enforcing, true), None);
        assert_eq!(mismatch(true, &pending, true), None);
        assert_eq!(mismatch(false, &disabled, false), None);
        assert_eq!(
            mismatch(true, &disabled, false),
            Some(Mismatch::NotEnrolled)
        );
        assert_eq!(mismatch(false, &pending, true), Some(Mismatch::KeysRemain));
        assert_eq!(
            mismatch(false, &enforcing, true),
            Some(Mismatch::EnforcingWhileDisabled)
        );
    }

    #[test]
    fn mismatch_ignores_foreign_keys_when_disabled() {
        // ARRANGE
        let foreign_pk = State {
            enabled: false,
            setup_mode: false,
            pk_present: true,
        };

        // ACT
        let mismatch = mismatch(false, &foreign_pk, false);

        // ASSERT
        assert_eq!(
            mismatch, None,
            "foreign keys must not report a leftover key"
        );
    }

    #[test]
    fn platform_key_matches_finds_our_x509_certificate() {
        // ARRANGE
        let certificate = b"our-x509-certificate";
        let store = Fake::ready().with(PK, x509_signature_list(certificate));

        // ACT
        let ours = platform_key_matches(&store, certificate).expect("match our certificate");
        let foreign = platform_key_matches(&store, b"microsoft-certificate")
            .expect("match foreign certificate");

        // ASSERT
        assert!(ours, "our certificate must match the enrolled PK");
        assert!(!foreign, "another certificate must not match");
    }

    #[test]
    fn platform_key_matches_rejects_missing_and_malformed_keys() {
        // ARRANGE
        let missing = Fake::ready();
        let malformed = Fake::ready().with(PK, vec![1, 2, 3]);

        // ACT
        let missing_match = platform_key_matches(&missing, b"certificate").expect("missing PK");
        let malformed_match =
            platform_key_matches(&malformed, b"certificate").expect("malformed PK");

        // ASSERT
        assert!(!missing_match, "a missing PK must not match");
        assert!(!malformed_match, "a malformed PK must not match");
    }
}
