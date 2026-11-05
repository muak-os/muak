//! Integration tests for mount and swap table parsing.

pub mod common;

#[cfg(test)]
mod tests {
    use blockdev::mounts;

    use crate::common::Fixture;

    #[test]
    fn reads_the_mount_table() {
        // ARRANGE
        let fixture = Fixture::new();
        fixture.mounts("/dev/sda1 /run/state ext4 rw,relatime 0 0\nproc /proc proc rw 0 0\n");

        // ACT
        let table = mounts::read(&fixture.roots().mounts);

        // ASSERT
        assert_eq!(table.len(), 2);
        let first = table.first().expect("first mount");
        assert_eq!(first.device, "/dev/sda1");
        assert_eq!(first.mount_point, "/run/state");
    }

    #[test]
    fn matches_a_mount_point_by_exact_device() {
        // ARRANGE
        let fixture = Fixture::new();
        let device = fixture.dev_node("sda1");
        fixture.mounts(&format!(
            "{} /run/state ext4 rw 0 0\n",
            device.to_string_lossy()
        ));
        let table = mounts::read(&fixture.roots().mounts);

        // ACT
        let found = mounts::of(&table, &device.to_string_lossy()).to_owned();
        let missing = mounts::of(&table, "/dev/sdz1").to_owned();

        // ASSERT
        assert_eq!(found, "/run/state");
        assert_eq!(missing, "");
    }

    #[test]
    fn matches_a_mount_point_through_a_canonical_path() {
        // ARRANGE
        let fixture = Fixture::new();
        let device = fixture.dev_node("sda1");
        let link = fixture.roots().dev.join("by-label-DATA");
        std::os::unix::fs::symlink(&device, &link).expect("device link");
        fixture.mounts(&format!(
            "{} /run/data btrfs rw 0 0\n",
            link.to_string_lossy()
        ));
        let table = mounts::read(&fixture.roots().mounts);

        // ACT
        let found = mounts::of(&table, &device.to_string_lossy()).to_owned();

        // ASSERT
        assert_eq!(found, "/run/data");
    }

    #[test]
    fn reads_the_swap_table() {
        // ARRANGE
        let fixture = Fixture::new();
        fixture.swaps(
            "Filename\tType\tSize\tUsed\tPriority\n/dev/sda1 partition 1024 0 -2\n/swapfile\tfile\t8\t0\t-3\n",
        );

        // ACT
        let swaps = mounts::try_read_swaps(&fixture.roots().swaps).expect("swaps");

        // ASSERT
        assert_eq!(swaps.len(), 2);
        assert_eq!(swaps.first().expect("first swap").device, "/dev/sda1");
        assert_eq!(swaps.last().expect("last swap").device, "/swapfile");
    }

    #[test]
    fn missing_tables_are_empty() {
        // ARRANGE
        let fixture = Fixture::new();
        let missing = fixture.roots().mounts.with_file_name("missing");

        // ACT
        let table = mounts::read(&missing);
        let swaps = mounts::try_read_swaps(&missing);

        // ASSERT
        assert_eq!(table, Vec::<mounts::Mount>::new());
        assert_eq!(swaps.expect("missing table"), Vec::<mounts::Swap>::new());
    }

    #[test]
    fn an_unreadable_swap_table_is_an_error() {
        // ARRANGE
        let fixture = Fixture::new();
        let path = fixture.roots().swaps;
        std::fs::remove_file(&path).expect("remove swap table");
        std::fs::create_dir_all(&path).expect("swap table directory");

        // ACT
        let swaps = mounts::try_read_swaps(&path);

        // ASSERT
        assert!(swaps.is_err(), "an unreadable swap table must be reported");
    }
}
