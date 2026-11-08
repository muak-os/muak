//! Integration tests for block device path helpers and size probing.

#[cfg(test)]
mod tests {
    use std::os::fd::AsFd as _;
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
    fn belongs_to_matches_only_the_disk_family_separator() {
        // ARRANGE
        let cases = [
            ("/dev/sda", "/dev/sda", true),
            ("/dev/sda1", "/dev/sda", true),
            ("/dev/sda12", "/dev/sda", true),
            ("/dev/sdaa1", "/dev/sda", false),
            ("/dev/nvme0n1p1", "/dev/nvme0n1", true),
            ("/dev/nvme0n1p12", "/dev/nvme0n1", true),
            ("/dev/nvme0n11", "/dev/nvme0n1", false),
            ("/dev/mmcblk0p2", "/dev/mmcblk0", true),
            ("/dev/mmcblk0boot0", "/dev/mmcblk0", false),
            ("/dev/md0p1", "/dev/md0", true),
            ("/dev/md01", "/dev/md0", false),
            ("/dev/loop0p1", "/dev/loop0", true),
            ("/dev/pmem0p1", "/dev/pmem0", true),
            ("/dev/sdap1", "/dev/sda", false),
            ("/dev/mapper/muak-state", "/dev/sda", false),
            ("tmpfs", "/dev/sda", false),
        ];

        // ACT
        for &(device, disk, expected) in &cases {
            let matched = device::belongs_to(device, disk);

            // ASSERT
            assert_eq!(matched, expected, "{device} against {disk}");
        }
    }

    #[test]
    fn size_returns_byte_length_of_regular_file() {
        // ARRANGE
        let file = tempfile::NamedTempFile::new().expect("temp file");
        file.as_file().set_len(1234).expect("resize");

        // ACT
        let size = device::size(file.as_file()).expect("size probe");

        // ASSERT
        assert_eq!(size, 1234, "regular files report their metadata len");
    }

    #[test]
    fn size_accepts_a_bare_descriptor() {
        // ARRANGE
        let file = tempfile::NamedTempFile::new().expect("temp file");
        file.as_file().set_len(4321).expect("resize");

        // ACT
        let size = device::size(&file.as_file().as_fd()).expect("size probe");

        // ASSERT
        assert_eq!(size, 4321, "fd-only callers need no File");
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
