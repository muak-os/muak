//! Firmware inventory.

use crate::nvram::VariableStore;
use crate::platform::Platform;
use crate::{esrt, identity};

/// Snapshot of platform firmware and its updatable components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inventory {
    /// System firmware identity, when the platform exposes one.
    pub platform: Option<identity::Identity>,
    /// Updatable firmware components reported by the firmware.
    pub components: Vec<esrt::Component>,
}

/// Collects firmware inventory from a platform implementation.
#[must_use]
pub fn collect<S: VariableStore + Platform>(system: &S) -> Inventory {
    Inventory {
        platform: identity::collect(system, system),
        components: esrt::collect(system),
    }
}
