//! Distro-owned disk layouts, resolved by name from board requirements.

use crate::error::DiskError;
use crate::plan::{PartitionSpec, Plan, Size};
use crate::role::Role;
use crate::sizes::{EFI_SIZE, STATE_SIZE};

/// A distro-owned disk layout applied by the installer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Standard UEFI/GPT layout: ESP, STATE and EPHEMERAL filling the disk.
    Uefi,
}

impl Layout {
    /// Every disk layout this build knows.
    pub const ALL: &'static [Layout] = &[Layout::Uefi];

    /// Canonical name of this layout, used by the `dev.muak.disk` annotation.
    #[must_use = "callers need the name to label the layout"]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Uefi => "uefi",
        }
    }

    /// Resolves a layout by its canonical name.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError::UnknownLayout`] when no known layout carries
    /// `name`.
    pub fn by_name(name: &str) -> Result<Self, DiskError> {
        Self::ALL
            .iter()
            .copied()
            .find(|layout| layout.name() == name)
            .ok_or_else(|| DiskError::UnknownLayout {
                name: name.to_owned(),
                known: Self::ALL
                    .iter()
                    .map(|layout| layout.name())
                    .collect::<Vec<_>>()
                    .join(", "),
            })
    }

    /// System plan of this layout: ESP, STATE and EPHEMERAL filling the disk.
    #[must_use = "dropping it discards the planned layout"]
    pub fn plan(self) -> Plan {
        match self {
            Self::Uefi => Plan::wiped(vec![
                spec(Role::Esp, Size::Fixed(EFI_SIZE)),
                spec(Role::State, Size::Fixed(STATE_SIZE)),
                spec(Role::Ephemeral, Size::Fill),
            ]),
        }
    }
}

fn spec(role: Role, size: Size) -> PartitionSpec {
    PartitionSpec::create(role, role.gpt_name(), role.type_guid(), size)
}
