//! Muak role membership derived from the block device inventory.

use ::disk::role::Role;
use blockdev::{discover, types};

/// Returns true when `disk` carries the Muak STATE partition.
pub(crate) fn is_system(disk: &types::Disk) -> bool {
    has(disk, Role::State)
}

/// Returns true when `disk` carries the Muak DATA partition.
pub(crate) fn is_data(disk: &types::Disk) -> bool {
    has(disk, Role::Data)
}

/// Returns true when a partition is mounted or carries a Muak role name.
pub(crate) fn in_use(partition: &types::Partition) -> bool {
    !partition.mount_point.is_empty()
        || matches!(
            Role::from_gpt_name(&partition.gpt_name),
            Some(Role::State | Role::Ephemeral | Role::Data)
        )
}

/// Returns the device path of the partition carrying `role`.
pub(crate) fn device(role: Role) -> Option<String> {
    discover::find_by_partname(role.gpt_name())
        .into_iter()
        .next()
}

fn has(disk: &types::Disk, role: Role) -> bool {
    disk.partitions
        .iter()
        .any(|partition| Role::from_gpt_name(&partition.gpt_name) == Some(role))
}

#[cfg(test)]
mod tests {
    use blockdev::types::{Disk, Partition};

    use super::*;

    fn disk(partitions: Vec<Partition>) -> Disk {
        Disk {
            partitions,
            ..Disk::default()
        }
    }

    fn partition(gpt_name: &str, mount_point: &str) -> Partition {
        Partition {
            gpt_name: gpt_name.to_owned(),
            mount_point: mount_point.to_owned(),
            ..Partition::default()
        }
    }

    #[test]
    fn system_and_data_membership_follow_the_gpt_names() {
        // ARRANGE
        let system = disk(vec![partition(Role::State.gpt_name(), "")]);
        let data = disk(vec![partition(Role::Data.gpt_name(), "")]);
        let both = disk(vec![
            partition(Role::State.gpt_name(), ""),
            partition(Role::Data.gpt_name(), ""),
        ]);

        // ACT
        let results = [
            (is_system(&system), is_data(&system)),
            (is_system(&data), is_data(&data)),
            (is_system(&both), is_data(&both)),
        ];

        // ASSERT
        assert_eq!(results, [(true, false), (false, true), (true, true)]);
    }

    #[test]
    fn only_whole_gpt_names_count_as_muak_partitions() {
        // ARRANGE
        let renamed = disk(vec![
            partition("STATE_OLD", ""),
            partition("DATA_OLD", ""),
            partition("EPHEMERAL_OLD", ""),
        ]);

        // ACT
        let results = (
            is_system(&renamed),
            is_data(&renamed),
            in_use(&renamed.partitions[0]),
            in_use(&renamed.partitions[2]),
        );

        // ASSERT
        assert_eq!(results, (false, false, false, false));
    }

    #[test]
    fn a_partition_is_in_use_when_mounted_or_named_for_a_muak_role() {
        // ARRANGE
        let mounted = partition("", "/mnt");
        let state = partition(Role::State.gpt_name(), "");
        let ephemeral = partition(Role::Ephemeral.gpt_name(), "");
        let data = partition(Role::Data.gpt_name(), "");
        let esp = partition(Role::Esp.gpt_name(), "");
        let bare = partition("", "");

        // ACT
        let checked = [
            in_use(&mounted),
            in_use(&state),
            in_use(&ephemeral),
            in_use(&data),
            in_use(&esp),
            in_use(&bare),
        ];

        // ASSERT
        assert_eq!(
            checked,
            [true, true, true, true, false, false],
            "mounted, STATE, EPHEMERAL and DATA are in use; the ESP is not"
        );
    }
}
