//! Integration tests for the executor plan model.

#[cfg(test)]
mod tests {
    use disk::plan::{PartitionSpec, Plan, Size};
    use disk::role::Role;
    use disk::sizes::ESP_TYPE_GUID;
    use parttable::gpt::partition::LINUX_FS_GUID;

    #[test]
    fn create_spec_sets_role_name_and_size() {
        // ARRANGE
        let role = Role::Esp;
        let name = "EFI";
        let size = Size::Fixed(512);

        // ACT
        let spec = PartitionSpec::create(role, name, ESP_TYPE_GUID, size);

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
}
