//! Integration tests for the sysfs disk inventory.

pub mod common;

#[cfg(test)]
mod tests {
    use std::os::unix::fs::FileExt as _;
    use std::path::Path;

    use blockdev::error::BlockdevError;
    use blockdev::kernel::Kernel as _;
    use blockdev::linux::Linux;
    use blockdev::types::Disk;
    use parttable::gpt;

    use crate::common::Fixture;

    const LUKS_MAGIC: [u8; 6] = *b"LUKS\xba\xbe";
    const BTRFS_MAGIC_OFFSET: u64 = 0x10040;
    const BTRFS_MAGIC: [u8; 8] = *b"_BHRfS_M";

    fn write_magic(device: &Path, offset: u64, magic: &[u8]) {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(device)
            .expect("device node");
        let end = offset
            .checked_add(u64::try_from(magic.len()).expect("magic length"))
            .expect("magic offset");
        file.set_len(end).expect("device size");
        file.write_all_at(magic, offset).expect("magic");
    }

    #[test]
    fn lists_the_physical_disks_sorted_by_name() {
        // ARRANGE
        let fixture = Fixture::new();
        fixture.disk("sdb", 2048);
        fixture.disk("sda", 2048);
        fixture.disk("sdz", 0);
        for ignored in ["loop0", "dm-0", "ram0", "sr0"] {
            fixture.disk(ignored, 2048);
        }

        // ACT
        let disks = Linux::rooted(&fixture.roots()).disks().expect("disks");

        // ASSERT
        let names: Vec<&str> = disks.iter().map(|disk| disk.name.as_str()).collect();
        assert_eq!(names, ["sda", "sdb"], "pseudo devices and empty disks");
    }

    #[test]
    fn reads_the_disk_facts() {
        // ARRANGE
        let fixture = Fixture::new();
        fixture.disk("sda", 4096);
        fixture.attribute("sda", "removable", "1\n");
        fixture.attribute("sda", "ro", "1\n");
        fixture.attribute("sda", "queue/rotational", "1\n");
        fixture.attribute("sda", "device/model", "Samsung SSD\n");
        fixture.attribute("sda", "device/serial", "SN123\n");
        fixture.attribute("sda", "wwid", "naa.5000\n");

        // ACT
        let disks = Linux::rooted(&fixture.roots()).disks().expect("disks");
        let disk: &Disk = disks.first().expect("disk");

        // ASSERT
        assert_eq!(disk.path, fixture.roots().dev.join("sda").to_string_lossy());
        assert_eq!(disk.size_bytes, 4096 * gpt::plan::SECTOR_SIZE);
        assert_eq!(disk.model, "Samsung SSD");
        assert_eq!(disk.serial, "SN123");
        assert_eq!(disk.wwid, "naa.5000");
        assert_eq!(disk.subsystem, "scsi");
        assert!(disk.rotational, "rotational media");
        assert!(disk.removable, "removable media");
        assert!(disk.read_only, "read only media");
    }

    #[test]
    fn discovers_the_partitions_of_a_disk() {
        // ARRANGE
        let fixture = Fixture::new();
        fixture.disk("sda", 8192);
        fixture.partition("sda", "sda2", 2, "BULK", 4096, 2048);
        fixture.partition("sda", "sda1", 1, "STATE", 2048, 1024);
        let state = fixture.roots().dev.join("sda1");
        fixture.mounts(&format!(
            "{} /run/state ext4 rw,relatime 0 0\n",
            state.to_string_lossy()
        ));

        // ACT
        let disks = Linux::rooted(&fixture.roots()).disks().expect("disks");
        let partitions = &disks.first().expect("disk").partitions;

        // ASSERT
        assert_eq!(partitions.len(), 2);
        let first = partitions.first().expect("first partition");
        assert_eq!(first.number, 1, "partitions are sorted by number");
        assert_eq!(first.start_sector, 2048);
        assert_eq!(first.size_bytes, 1024 * gpt::plan::SECTOR_SIZE);
        assert_eq!(first.name, "sda1");
        assert_eq!(first.path, state.to_string_lossy());
        assert_eq!(first.gpt_name, "STATE");
        assert_eq!(first.mount_point, "/run/state");
        assert_eq!(first.fstype, "");
    }

    #[test]
    fn detects_an_ext4_filesystem_on_a_partition() {
        // ARRANGE
        let fixture = Fixture::new();
        fixture.disk("sda", 8192);
        fixture.partition("sda", "sda1", 1, "STATE", 2048, 1024);
        let device = fixture.roots().dev.join("sda1");
        write_magic(&device, 0x438, &[0x53, 0xEF]);

        // ACT
        let disks = Linux::rooted(&fixture.roots()).disks().expect("disks");
        let fstype = &disks
            .first()
            .expect("disk")
            .partitions
            .first()
            .expect("partition")
            .fstype;

        // ASSERT
        assert_eq!(fstype, "ext4");
    }

    #[test]
    fn reports_the_filesystem_inside_a_luks_mapping() {
        // ARRANGE
        let fixture = Fixture::new();
        fixture.disk("sda", 8192);
        fixture.partition("sda", "sda1", 1, "STATE", 2048, 1024);
        write_magic(&fixture.roots().dev.join("sda1"), 0, &LUKS_MAGIC);
        fixture.holder("sda/sda1", "dm-0");
        fixture.mapping("dm-0", "muak-state");
        let mapped = fixture.mapped_node("muak-state");
        write_magic(&mapped, BTRFS_MAGIC_OFFSET, &BTRFS_MAGIC);

        // ACT
        let disks = Linux::rooted(&fixture.roots()).disks().expect("disks");
        let fstype = &disks
            .first()
            .expect("disk")
            .partitions
            .first()
            .expect("partition")
            .fstype;

        // ASSERT
        assert_eq!(fstype, "btrfs");
    }

    #[test]
    fn reports_an_unmapped_luks_partition_as_encrypted() {
        // ARRANGE
        let fixture = Fixture::new();
        fixture.disk("sda", 8192);
        fixture.partition("sda", "sda1", 1, "STATE", 2048, 1024);
        write_magic(&fixture.roots().dev.join("sda1"), 0, &LUKS_MAGIC);

        // ACT
        let disks = Linux::rooted(&fixture.roots()).disks().expect("disks");
        let fstype = &disks
            .first()
            .expect("disk")
            .partitions
            .first()
            .expect("partition")
            .fstype;

        // ASSERT
        assert_eq!(fstype, "crypto_LUKS");
    }

    #[test]
    fn skips_a_disk_with_an_unparsable_attribute() {
        // ARRANGE
        let fixture = Fixture::new();
        fixture.disk("sda", 2048);
        fixture.disk("sdb", 2048);
        fixture.attribute("sdb", "size", "not-a-number\n");

        // ACT
        let disks = Linux::rooted(&fixture.roots()).disks().expect("disks");

        // ASSERT
        let names: Vec<&str> = disks.iter().map(|disk| disk.name.as_str()).collect();
        assert_eq!(names, ["sda"], "an unreadable disk is skipped");
    }

    #[test]
    fn a_missing_block_root_is_an_error() {
        // ARRANGE
        let fixture = Fixture::new();
        let mut roots = fixture.roots();
        roots.block = roots.block.join("missing");

        // ACT
        let error = Linux::rooted(&roots).disks().unwrap_err();

        // ASSERT
        assert!(
            matches!(error, BlockdevError::MissingRoot(_)),
            "a missing sysfs must be reported, got {error}"
        );
    }
}
