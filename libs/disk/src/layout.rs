//! Distro-owned disk layouts, resolved by name from board requirements.

use crate::error::DiskError;
use crate::plan::{self, PartitionSpec, Plan, Size};
use crate::role::Role;

/// A distro-owned disk layout applied by the installer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Standard UEFI/GPT layout: ESP and STATE, then DATA filling the disk.
    Uefi,
}

impl Layout {
    /// Every disk layout this build knows.
    pub const ALL: &'static [Layout] = &[Layout::Uefi];

    /// Canonical name of this layout, used by the `dev.muak.disk` annotation.
    #[must_use]
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

    /// System plan of this layout: ESP and STATE, then DATA filling the disk.
    #[must_use]
    pub fn plan(self) -> Plan {
        match self {
            Self::Uefi => Plan::wiped(vec![
                spec(Role::Esp, Size::Fixed(plan::EFI_SIZE)),
                spec(Role::State, Size::Fixed(plan::STATE_SIZE)),
                spec(Role::Data, Size::Fill),
            ]),
        }
    }
}

/// Creates a managed partition spec carrying its role's name and type GUID.
fn spec(role: Role, size: Size) -> PartitionSpec {
    PartitionSpec::create(role, role.gpt_name(), role.type_guid(), size)
}

#[cfg(test)]
mod tests {
    use parttable::gpt::partition::LINUX_FS_GUID;

    use super::*;
    use crate::plan::ESP_TYPE_GUID;

    #[test]
    fn by_name_resolves_known_layouts() {
        // ARRANGE / ACT / ASSERT
        for layout in Layout::ALL {
            assert_eq!(
                Layout::by_name(layout.name()).expect("known layout"),
                *layout,
                "every known name must resolve"
            );
        }
    }

    #[test]
    fn by_name_rejects_unknown_layouts_with_known_list() {
        // ARRANGE / ACT
        let error = Layout::by_name("apple").expect_err("unknown layout must fail");

        // ASSERT
        assert!(
            error.to_string().contains("uefi"),
            "error must list known layouts: {error}"
        );
    }

    #[test]
    fn uefi_plan_is_frozen() {
        // ARRANGE / ACT
        let plan = Layout::Uefi.plan();

        // ASSERT
        assert!(plan.wipe, "the system disk must be wiped");
        let roles: Vec<_> = plan.partitions.iter().map(|spec| spec.role).collect();
        let names: Vec<_> = plan
            .partitions
            .iter()
            .map(|spec| spec.name.clone())
            .collect();
        let sizes: Vec<_> = plan.partitions.iter().map(|spec| spec.size).collect();
        let guids: Vec<_> = plan.partitions.iter().map(|spec| spec.type_guid).collect();
        assert_eq!(
            roles,
            vec![Some(Role::Esp), Some(Role::State), Some(Role::Data)]
        );
        assert_eq!(names, vec!["EFI", "STATE", "DATA"]);
        assert_eq!(
            sizes,
            vec![
                Size::Fixed(plan::EFI_SIZE),
                Size::Fixed(plan::STATE_SIZE),
                Size::Fill
            ]
        );
        assert_eq!(guids, vec![ESP_TYPE_GUID, LINUX_FS_GUID, LINUX_FS_GUID]);
    }
}
