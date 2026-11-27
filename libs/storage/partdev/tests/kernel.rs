//! Integration tests for the kernel partition table operations.

pub mod common;

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::fs::File;

    use partdev::error::Result;
    use partdev::kernel::Kernel;
    use partdev::linux::Linux;
    use partdev::{manage, table};

    use super::common::{Fixture, SECTOR_BYTES};

    #[derive(Default)]
    struct Fake {
        rescans: Cell<u32>,
    }

    impl Kernel for Fake {
        fn rescan(&self, _device: &str) -> Result<()> {
            self.rescans.set(self.rescans.get().saturating_add(1));
            Ok(())
        }

        fn size(&self, file: &File) -> Result<u64> {
            Ok(file.metadata()?.len())
        }
    }

    #[test]
    fn size_reports_the_backing_file_length() {
        // ARRANGE
        let fixture = Fixture::new(2048);

        // ACT
        let size = Linux::new().size(fixture.file()).expect("size");

        // ASSERT
        assert_eq!(size, fixture.sectors().saturating_mul(SECTOR_BYTES));
    }

    #[test]
    fn delete_partitions_in_uses_the_supplied_kernel() {
        // ARRANGE
        let fixture = Fixture::new(2048);
        fixture.write_gpt(&["EFI", "STATE"]);
        let kernel = Fake::default();

        // ACT
        manage::delete_partitions_in(&kernel, fixture.path_str(), &[2]).expect("delete in");

        // ASSERT
        let gpt = table::read(fixture.file()).expect("GPT must stay readable");
        assert!(!gpt.is_partition_used(2), "deleted partition must be gone");
        assert_eq!(
            kernel.rescans.get(),
            1,
            "the supplied kernel must be asked to rescan once"
        );
    }

    #[test]
    fn rescan_on_a_regular_file_errors_instead_of_panicking() {
        // ARRANGE
        let fixture = Fixture::new(2048);

        // ACT
        let result = Linux::new().rescan(fixture.path_str());

        // ASSERT
        assert!(
            result.is_err(),
            "the kernel rejects a table re-read on a regular file"
        );
    }
}
