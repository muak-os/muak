//! Integration tests for GPT name lookup and parent disk resolution.

pub mod common;

#[cfg(test)]
mod tests {
    use blockdev::discover;

    use crate::common::Fixture;

    #[test]
    fn find_by_partname_matches_the_whole_name_only() {
        // ARRANGE
        let fixture = Fixture::new();
        fixture.disk("sda", 8192);
        fixture.partition("sda", "sda1", 1, "EFI", 2048, 512);
        fixture.partition("sda", "sda2", 2, "EFI_OLD", 4096, 512);
        fixture.partition("sda", "sda3", 3, "DATA", 6144, 512);

        // ACT
        let devices = discover::find_by_partname_in(&fixture.roots(), "EFI");

        // ASSERT
        assert_eq!(
            devices,
            [fixture
                .roots()
                .dev
                .join("sda1")
                .to_string_lossy()
                .into_owned()],
            "a longer name must not match"
        );
    }

    #[test]
    fn the_first_partname_wins_when_it_is_repeated() {
        // ARRANGE
        let fixture = Fixture::new();
        fixture.disk("sda", 8192);
        fixture.partition("sda", "sda1", 1, "EFI", 2048, 512);
        fixture.attribute(
            "sda/sda1",
            "uevent",
            "DEVNAME=sda1\nPARTNAME=FIRST\nPARTNAME=SECOND\n",
        );

        // ACT
        let first = discover::find_by_partname_in(&fixture.roots(), "FIRST");
        let second = discover::find_by_partname_in(&fixture.roots(), "SECOND");

        // ASSERT
        assert_eq!(first.len(), 1);
        assert!(second.is_empty(), "the first PARTNAME wins");
    }

    #[test]
    fn find_by_partname_returns_nothing_for_an_unknown_name() {
        // ARRANGE
        let fixture = Fixture::new();
        fixture.disk("sda", 8192);
        fixture.partition("sda", "sda1", 1, "STATE", 2048, 512);

        // ACT
        let devices = discover::find_by_partname_in(&fixture.roots(), "DATA");

        // ASSERT
        assert_eq!(devices, Vec::<String>::new());
    }

    #[test]
    fn parent_disk_resolves_a_partition_device() {
        // ARRANGE
        let fixture = Fixture::new();
        fixture.disk("nvme0n1", 8192);
        fixture.partition("nvme0n1", "nvme0n1p2", 2, "STATE", 2048, 512);
        let device = fixture.roots().dev.join("nvme0n1p2");

        // ACT
        let disk = discover::parent_disk_in(&fixture.roots(), &device.to_string_lossy());

        // ASSERT
        assert_eq!(disk.as_deref(), Some("nvme0n1"));
    }

    #[test]
    fn parent_disk_returns_nothing_for_an_unknown_device() {
        // ARRANGE
        let fixture = Fixture::new();

        // ACT
        let disk = discover::parent_disk_in(&fixture.roots(), "/dev/sdz9");

        // ASSERT
        assert_eq!(disk, None);
    }
}
