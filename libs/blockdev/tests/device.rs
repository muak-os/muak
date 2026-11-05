//! Integration tests for block device path helpers.

#[cfg(test)]
mod tests {
    use std::path::Path;

    use blockdev::device;

    #[test]
    fn partition_path_follows_the_kernel_naming_rule() {
        // ARRANGE
        let disks = ["/dev/sda", "/dev/vda"];
        let named = [("/dev/nvme0n1", 1), ("/dev/mmcblk0", 2)];

        // ACT
        let sata = device::partition_path(disks[0], 3);
        let virtio = device::partition_path(disks[1], 1);
        let nvme = device::partition_path(named[0].0, named[0].1);
        let mmcblk = device::partition_path(named[1].0, named[1].1);

        // ASSERT
        assert_eq!(sata, "/dev/sda3");
        assert_eq!(virtio, "/dev/vda1");
        assert_eq!(nvme, "/dev/nvme0n1p1");
        assert_eq!(mmcblk, "/dev/mmcblk0p2");
    }

    #[test]
    fn looks_like_partition_flags_numbered_disk_nodes() {
        // ARRANGE
        let partitions = ["/dev/sda1", "/dev/vda2", "/dev/hda12"];
        let disks = ["/dev/sda", "/dev/nvme0n1", "/dev/mapper/data"];

        // ACT
        let partition_flags: Vec<bool> = partitions
            .iter()
            .map(|path| device::looks_like_partition(path))
            .collect();
        let disk_flags: Vec<bool> = disks
            .iter()
            .map(|path| device::looks_like_partition(path))
            .collect();

        // ASSERT
        assert_eq!(partition_flags, [true, true, true], "numbered disk nodes");
        assert_eq!(disk_flags, [false, false, false], "whole disk nodes");
    }

    #[test]
    fn is_block_rejects_regular_files() {
        // ARRANGE
        let file = tempfile::NamedTempFile::new().expect("temp file");

        // ACT
        let block = device::is_block(file.path());
        let missing = device::is_block(Path::new("/dev/muak-missing"));

        // ASSERT
        assert!(!block, "a regular file is not a block device");
        assert!(!missing, "a missing path is not a block device");
    }
}
