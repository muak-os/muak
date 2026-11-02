//! Conversion of the install plan into its protobuf representation.

use ::disk::plan::Size;

use super::checks;
use super::inventory::to_disk_info;
use super::proto::provision::{InstallPlan, PlannedPartition};
use crate::install::plan::{InstallPlan as DomainPlan, PlannedPartition as DomainPartition};

/// Converts an install plan into its protobuf representation.
pub(crate) fn to_install_plan(plan: DomainPlan) -> InstallPlan {
    InstallPlan {
        system_disk: plan.system.map(to_disk_info),
        data_disk: plan.data.map(to_disk_info),
        shared_data: plan.shared_data,
        wipe: plan.wipe,
        partitions: plan
            .partitions
            .into_iter()
            .map(to_planned_partition)
            .collect(),
        checks: plan.checks.into_iter().map(checks::to_proto).collect(),
    }
}

fn to_planned_partition(partition: DomainPartition) -> PlannedPartition {
    let (size_bytes, fill) = match partition.size {
        Size::Fixed(bytes) => (bytes, false),
        Size::Fill => (0, true),
    };

    PlannedPartition {
        disk: partition.disk,
        role: partition.role,
        name: partition.name,
        size_bytes,
        fill,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_sizes_carry_their_byte_count() {
        // ARRANGE
        let partition = DomainPartition {
            disk: "/dev/sda".to_owned(),
            role: "EFI".to_owned(),
            name: "EFI".to_owned(),
            size: Size::Fixed(512),
        };

        // ACT
        let proto = to_planned_partition(partition);

        // ASSERT
        assert_eq!(proto.disk, "/dev/sda");
        assert_eq!(proto.role, "EFI");
        assert_eq!(proto.size_bytes, 512);
        assert!(!proto.fill);
    }

    #[test]
    fn fill_partitions_have_no_byte_count() {
        // ARRANGE
        let partition = DomainPartition {
            disk: "/dev/sda".to_owned(),
            role: "DATA".to_owned(),
            name: "DATA".to_owned(),
            size: Size::Fill,
        };

        // ACT
        let proto = to_planned_partition(partition);

        // ASSERT
        assert_eq!(proto.size_bytes, 0);
        assert!(proto.fill);
    }
}
