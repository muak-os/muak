//! Integration tests for device-level partition table queries.

pub mod common;

#[cfg(test)]
mod tests {
    use partdev::{manage, table};

    use super::common::{Fixture, read_at};

    #[test]
    fn is_non_empty_is_true_for_a_gpt() {
        // ARRANGE
        let fixture = Fixture::new(2048);
        fixture.write_gpt(&["EFI"]);

        // ACT
        let non_empty = manage::is_non_empty(fixture.path_str()).expect("probe");

        // ASSERT
        assert!(non_empty);
    }

    #[test]
    fn is_non_empty_is_true_for_an_mbr() {
        // ARRANGE
        let fixture = Fixture::new(2048);
        fixture.write_mbr();

        // ACT
        let non_empty = manage::is_non_empty(fixture.path_str()).expect("probe");

        // ASSERT
        assert!(non_empty);
    }

    #[test]
    fn is_non_empty_is_false_for_zeroes() {
        // ARRANGE
        let fixture = Fixture::new(2048);

        // ACT
        let non_empty = manage::is_non_empty(fixture.path_str()).expect("probe");

        // ASSERT
        assert!(!non_empty);
    }

    #[test]
    fn partition_number_resolves_a_gpt_name() {
        // ARRANGE
        let fixture = Fixture::new(2048);
        fixture.write_gpt(&["EFI", "STATE"]);

        // ACT
        let number = manage::partition_number(fixture.path_str(), "STATE").expect("lookup");

        // ASSERT
        assert_eq!(number, Some(2));
    }

    #[test]
    fn partition_number_is_none_for_an_absent_name() {
        // ARRANGE
        let fixture = Fixture::new(2048);
        fixture.write_gpt(&["EFI"]);

        // ACT
        let number = manage::partition_number(fixture.path_str(), "STATE").expect("lookup");

        // ASSERT
        assert_eq!(number, None);
    }

    #[test]
    fn delete_partitions_keeps_the_table_structurally_valid() {
        // ARRANGE
        let fixture = Fixture::new(2048);
        fixture.write_gpt(&["EFI", "STATE"]);

        // ACT
        // A table re-read needs a real block device, so the kernel step fails on a temp file;
        // the table is committed before that step and must survive.
        let result = manage::delete_partitions(fixture.path_str(), &[2]);

        // ASSERT
        assert!(
            result.is_err(),
            "the kernel must reject a table re-read on a temp file"
        );
        let gpt = table::read(fixture.file()).expect("GPT must stay readable");
        assert!(!gpt.is_partition_used(2), "deleted partition must be gone");
        assert!(gpt.is_partition_used(1), "kept partition must survive");

        let sector = read_at(fixture.file(), 0, 512);
        assert_eq!(
            sector.get(510..512),
            Some(&[0x55, 0xAA][..]),
            "protective MBR boot signature must survive the commit"
        );
        assert_eq!(
            sector.get(450),
            Some(&0xEE),
            "protective MBR partition type must survive the commit"
        );
    }
}
