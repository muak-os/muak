//! Integration tests for the functional partition roles.

#[cfg(test)]
mod tests {
    use disk::role::Role;
    use disk::sizes::ESP_TYPE_GUID;
    use parttable::gpt::partition::LINUX_FS_GUID;
    use serde::Deserialize;

    #[test]
    fn gpt_names_are_stable() {
        // ARRANGE
        let roles = [Role::Esp, Role::State, Role::Ephemeral];

        // ACT
        let names: Vec<_> = roles.iter().map(|role| role.gpt_name()).collect();

        // ASSERT
        assert_eq!(names, vec!["EFI", "STATE", "EPHEMERAL"]);
    }

    #[test]
    fn type_guids_are_stable() {
        // ARRANGE
        let roles = [Role::Esp, Role::State, Role::Ephemeral];

        // ACT
        let guids: Vec<_> = roles.iter().map(|role| role.type_guid()).collect();

        // ASSERT
        assert_eq!(guids, vec![ESP_TYPE_GUID, LINUX_FS_GUID, LINUX_FS_GUID]);
    }

    #[test]
    fn dm_names_are_stable() {
        // ARRANGE
        let roles = [Role::State, Role::Ephemeral];

        // ACT
        let names: Vec<_> = roles.iter().map(|role| role.dm_name()).collect();

        // ASSERT
        assert_eq!(names, vec!["muak-state", "muak-ephemeral"]);
    }

    #[test]
    fn data_is_not_a_role() {
        // ARRANGE
        let name = "DATA";

        // ACT
        let role = Role::from_gpt_name(name);

        // ASSERT
        assert!(role.is_none(), "the DATA partition role no longer exists");
    }

    #[test]
    fn serde_round_trip_uses_lowercase() {
        // ARRANGE
        #[derive(Deserialize)]
        struct Wrapper {
            role: Role,
        }

        // ACT
        let parsed = toml::from_str::<Wrapper>("role = \"state\"").expect("parse");

        // ASSERT
        assert_eq!(parsed.role, Role::State, "role must deserialize lowercase");
    }
}
