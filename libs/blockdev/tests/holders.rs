//! Integration tests for device-mapper holder resolution.

pub mod common;

#[cfg(test)]
mod tests {
    use blockdev::holders;

    use crate::common::Fixture;

    #[test]
    fn resolves_holders_to_their_mapping_names() {
        // ARRANGE
        let fixture = Fixture::new();
        fixture.disk("sda", 8192);
        fixture.partition("sda", "sda1", 1, "STATE", 2048, 512);
        fixture.holder("sda/sda1", "dm-0");
        fixture.mapping("dm-0", "muak-state");
        fixture.holder("sda/sda1", "luks-1234");

        // ACT
        let names = holders::try_names(&fixture.roots(), "sda/sda1").expect("holders");

        // ASSERT
        assert_eq!(names, ["luks-1234", "muak-state"]);
    }

    #[test]
    fn a_device_without_holders_reports_nothing() {
        // ARRANGE
        let fixture = Fixture::new();
        fixture.disk("sda", 8192);

        // ACT
        let names = holders::try_names(&fixture.roots(), "sda").expect("holders");

        // ASSERT
        assert_eq!(names, Vec::<String>::new());
    }

    #[test]
    fn an_unreadable_holders_directory_is_an_error() {
        // ARRANGE
        let fixture = Fixture::new();
        std::fs::write(fixture.roots().block.join("sda"), b"").expect("blocking file");

        // ACT
        let names = holders::try_names(&fixture.roots(), "sda");

        // ASSERT
        assert!(
            names.is_err(),
            "an unreadable holders directory must be reported"
        );
    }
}
