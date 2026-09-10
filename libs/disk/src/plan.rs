//! Declarative partition plan describing what a platform install places on disk.

use parttable::gpt::partition::LINUX_FS_GUID;
use serde::{Deserialize, Serialize};

use crate::role::Role;

/// GPT type GUID of the EFI System Partition (wire byte order).
pub const ESP_TYPE_GUID: [u8; 16] = [
    0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b,
];

/// Frozen size of the EFI System Partition.
pub const EFI_SIZE: u64 = 512 * 1024 * 1024;

/// Frozen size of the STATE partition.
pub const STATE_SIZE: u64 = 1024 * 1024 * 1024;

/// Size of a planned partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Size {
    /// Exact size in bytes.
    Fixed(u64),
    /// All remaining usable space on the disk.
    Fill,
}

/// One planned partition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartitionSpec {
    /// Functional role; `None` for foreign partitions kept by the platform.
    pub role: Option<Role>,
    /// GPT partition name.
    pub name: String,
    /// GPT partition type GUID.
    pub type_guid: [u8; 16],
    /// Partition size.
    pub size: Size,
}

impl PartitionSpec {
    /// Creates a partition spec for partitions we manage.
    #[must_use]
    pub fn create(role: Role, name: &str, type_guid: [u8; 16], size: Size) -> Self {
        Self {
            role: Some(role),
            name: name.to_owned(),
            type_guid,
            size,
        }
    }
}

/// The partition plan applied to one disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// Whether the disk is wiped before applying the plan.
    pub wipe: bool,
    /// Planned partitions, in placement order.
    pub partitions: Vec<PartitionSpec>,
}

impl Plan {
    /// Creates a plan that wipes the disk before placing the partitions.
    #[must_use]
    pub fn wiped(partitions: Vec<PartitionSpec>) -> Self {
        Self {
            wipe: true,
            partitions,
        }
    }
}

/// Builds the standard UEFI platform layout.
#[must_use]
pub fn uefi(shared_data: bool) -> (Plan, Option<Plan>) {
    let mut partitions = vec![
        PartitionSpec::create(
            Role::Esp,
            Role::Esp.gpt_name(),
            ESP_TYPE_GUID,
            Size::Fixed(EFI_SIZE),
        ),
        PartitionSpec::create(
            Role::State,
            Role::State.gpt_name(),
            LINUX_FS_GUID,
            Size::Fixed(STATE_SIZE),
        ),
    ];

    if shared_data {
        partitions.push(data_spec());
        (Plan::wiped(partitions), None)
    } else {
        (
            Plan::wiped(partitions),
            Some(Plan::wiped(vec![data_spec()])),
        )
    }
}

fn data_spec() -> PartitionSpec {
    PartitionSpec::create(Role::Data, Role::Data.gpt_name(), LINUX_FS_GUID, Size::Fill)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_spec_sets_role_name_and_size() {
        // ARRANGE / ACT
        let spec = PartitionSpec::create(Role::Esp, "EFI", ESP_TYPE_GUID, Size::Fixed(512));

        // ASSERT
        assert_eq!(spec.role, Some(Role::Esp), "role must be set");
        assert_eq!(spec.name, "EFI", "name must be kept");
        assert_eq!(spec.type_guid, ESP_TYPE_GUID, "type GUID must be kept");
        assert_eq!(spec.size, Size::Fixed(512), "size must be kept");
    }

    #[test]
    fn wiped_plan_sets_wipe_flag_and_keeps_order() {
        // ARRANGE
        let specs = vec![
            PartitionSpec::create(Role::Esp, "EFI", ESP_TYPE_GUID, Size::Fixed(512)),
            PartitionSpec::create(Role::State, "STATE", LINUX_FS_GUID, Size::Fill),
        ];

        // ACT
        let plan = Plan::wiped(specs.clone());

        // ASSERT
        assert!(plan.wipe, "wiped() must set the wipe flag");
        assert_eq!(plan.partitions, specs, "partition order must be kept");
    }

    #[test]
    fn standard_uefi_layout_is_frozen() {
        // ARRANGE / ACT
        let (system, data) = uefi(true);

        // ASSERT
        assert!(system.wipe, "the system disk must be wiped");
        let roles: Vec<_> = system.partitions.iter().map(|spec| spec.role).collect();
        let names: Vec<_> = system
            .partitions
            .iter()
            .map(|spec| spec.name.clone())
            .collect();
        let sizes: Vec<_> = system.partitions.iter().map(|spec| spec.size).collect();
        let guids: Vec<_> = system
            .partitions
            .iter()
            .map(|spec| spec.type_guid)
            .collect();
        assert_eq!(
            roles,
            vec![Some(Role::Esp), Some(Role::State), Some(Role::Data)]
        );
        assert_eq!(names, vec!["EFI", "STATE", "DATA"]);
        assert_eq!(
            sizes,
            vec![Size::Fixed(EFI_SIZE), Size::Fixed(STATE_SIZE), Size::Fill]
        );
        assert_eq!(guids, vec![ESP_TYPE_GUID, LINUX_FS_GUID, LINUX_FS_GUID]);
        assert_eq!(data, None, "shared DATA must not produce a data plan");
    }

    #[test]
    fn standard_uefi_split_layout_keeps_data_off_the_system_disk() {
        // ARRANGE / ACT
        let (system, data) = uefi(false);

        // ASSERT
        assert_eq!(
            system.partitions.len(),
            2,
            "system disk carries ESP + STATE only"
        );
        assert!(
            system
                .partitions
                .iter()
                .all(|spec| spec.role != Some(Role::Data))
        );
        let data = data.expect("split layout must produce a data plan");
        assert!(data.wipe, "the data disk must be wiped");
        assert_eq!(data.partitions.len(), 1, "the data disk carries DATA only");
        let spec = data.partitions.first().expect("data spec");
        assert_eq!(spec.role, Some(Role::Data), "data spec role must be DATA");
        assert_eq!(spec.size, Size::Fill, "data spec must fill the disk");
    }
}
