//! Integration tests for the child path stage codes.

#[cfg(test)]
mod tests {
    use sandbox::stage::Stage;

    const ALL: [Stage; 8] = [
        Stage::Namespaces,
        Stage::Mounts,
        Stage::Rootfs,
        Stage::Cgroup,
        Stage::Credentials,
        Stage::Seccomp,
        Stage::Limits,
        Stage::Exec,
    ];

    #[test]
    fn codes_are_unique_and_non_zero() {
        // ARRANGE
        let mut codes = [0_u16; 8];

        // ACT
        for (position, stage) in ALL.into_iter().enumerate() {
            codes[position] = stage.code();
        }

        // ASSERT
        for (position, code) in codes.into_iter().enumerate() {
            assert_ne!(code, 0, "stage {position} has the reserved zero code");
            assert!(
                !codes[..position].contains(&code),
                "stage {position} repeats code {code}"
            );
        }
    }

    #[test]
    fn from_code_round_trips_every_stage() {
        // ARRANGE
        let codes = ALL.map(Stage::code);

        // ACT
        let recovered = codes.map(Stage::from_code);

        // ASSERT
        for (position, stage) in ALL.into_iter().enumerate() {
            assert_eq!(recovered[position], Some(stage));
        }
    }

    #[test]
    fn unknown_codes_have_no_stage() {
        // ARRANGE
        let unknown = [0_u16, 99];

        // ACT
        let stages = unknown.map(Stage::from_code);

        // ASSERT
        assert_eq!(stages, [None, None]);
    }

    #[test]
    fn names_are_non_empty_and_distinct() {
        // ARRANGE
        let mut names = [""; 8];

        // ACT
        for (position, stage) in ALL.into_iter().enumerate() {
            names[position] = stage.name();
        }

        // ASSERT
        for (position, name) in names.into_iter().enumerate() {
            assert!(!name.is_empty(), "stage {position} has no name");
            assert!(
                !names[..position].contains(&name),
                "stage {position} repeats name {name}"
            );
        }
    }
}
