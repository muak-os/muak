//! Integration tests for BLKPG behaviour that a regular file can exercise.

pub mod common;

#[cfg(test)]
mod tests {
    use partdev::blkpg;

    use super::common::Fixture;

    #[test]
    fn delete_all_on_a_regular_file_is_a_noop() {
        // ARRANGE
        let fixture = Fixture::new(2048);

        // ACT
        let result = blkpg::delete_all(fixture.path_str());

        // ASSERT
        assert!(result.is_ok(), "no partition node exists to delete");
    }

    #[test]
    fn add_on_a_regular_file_errors_instead_of_panicking() {
        // ARRANGE
        let fixture = Fixture::new(2048);

        // ACT
        let result = blkpg::add(fixture.path_str(), 1, 2048, 4095);

        // ASSERT
        assert!(
            result.is_err(),
            "the kernel rejects BLKPG on a regular file"
        );
    }

    #[test]
    fn delete_on_a_regular_file_errors_instead_of_panicking() {
        // ARRANGE
        let fixture = Fixture::new(2048);

        // ACT
        let result = blkpg::delete(fixture.path_str(), 1);

        // ASSERT
        assert!(
            result.is_err(),
            "the kernel rejects BLKPG on a regular file"
        );
    }
}
