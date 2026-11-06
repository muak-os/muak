//! Integration tests for the distro-owned disk layouts.

pub mod common;

#[cfg(test)]
mod tests {
    use disk::layout::Layout;
    use disk::plan::Size;
    use disk::role::Role;
    use disk::sizes::{EFI_SIZE, EPHEMERAL_SIZE, ESP_TYPE_GUID, STATE_SIZE};
    use parttable::gpt::partition::LINUX_FS_GUID;

    use crate::common;

    #[test]
    fn by_name_resolves_known_layouts() {
        // ARRANGE
        let layouts = Layout::ALL;

        // ACT
        let resolved: Vec<_> = layouts
            .iter()
            .map(|layout| Layout::by_name(layout.name()).expect("known layout"))
            .collect();

        // ASSERT
        assert_eq!(resolved, layouts.to_vec(), "every known name must resolve");
    }

    #[test]
    fn by_name_rejects_unknown_layouts_with_known_list() {
        // ARRANGE
        let name = "apple";

        // ACT
        let error = Layout::by_name(name).expect_err("unknown layout must fail");

        // ASSERT
        assert!(
            error.to_string().contains("uefi"),
            "error must list known layouts: {error}"
        );
    }

    #[test]
    fn uefi_plan_is_frozen() {
        // ARRANGE
        let plan = common::uefi_plan();

        // ACT
        let roles: Vec<_> = plan.partitions.iter().map(|spec| spec.role).collect();
        let names: Vec<_> = plan
            .partitions
            .iter()
            .map(|spec| spec.name.clone())
            .collect();
        let sizes: Vec<_> = plan.partitions.iter().map(|spec| spec.size).collect();
        let guids: Vec<_> = plan.partitions.iter().map(|spec| spec.type_guid).collect();

        // ASSERT
        assert!(plan.wipe, "the system disk must be wiped");
        assert_eq!(
            roles,
            vec![
                Some(Role::Esp),
                Some(Role::State),
                Some(Role::Ephemeral),
                Some(Role::Data)
            ]
        );
        assert_eq!(names, vec!["EFI", "STATE", "EPHEMERAL", "DATA"]);
        assert_eq!(
            sizes,
            vec![
                Size::Fixed(EFI_SIZE),
                Size::Fixed(STATE_SIZE),
                Size::Fixed(EPHEMERAL_SIZE),
                Size::Fill
            ]
        );
        assert_eq!(
            guids,
            vec![ESP_TYPE_GUID, LINUX_FS_GUID, LINUX_FS_GUID, LINUX_FS_GUID]
        );
    }
}
