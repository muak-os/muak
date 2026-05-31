#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::os::unix::ffi::OsStringExt as _;
    use std::os::unix::fs::PermissionsExt as _;

    use esp::{Arch, EspFile, EspSpec};
    use fatfs::{FileSystem, FsOptions};

    #[test]
    fn public_api_build_and_populate_match() {
        // ARRANGE
        let spec = EspSpec::with_uki(
            Arch::X86_64,
            b"uki-payload".to_vec(),
            vec![EspFile {
                path: "overlays/rpi/config.txt".to_owned(),
                data: b"arm_64bit=1".to_vec(),
            }],
        );
        let dest = tempfile::tempdir().expect("temp dir must be created");

        // ACT
        let image = esp::build(&spec).expect("build must succeed");
        esp::populate(&spec, dest.path()).expect("populate must succeed");

        // ASSERT
        let mut cursor = Cursor::new(image);
        let fs = FileSystem::new(&mut cursor, FsOptions::new()).expect("FAT filesystem must open");
        let mut boot = fs
            .root_dir()
            .open_dir("EFI")
            .expect("EFI dir must exist")
            .open_dir("BOOT")
            .expect("BOOT dir must exist")
            .open_file("BOOTX64.EFI")
            .expect("boot file must exist");
        let mut boot_bytes = Vec::new();
        std::io::Read::read_to_end(&mut boot, &mut boot_bytes).expect("boot file must read");
        assert_eq!(boot_bytes, b"uki-payload");
        assert_eq!(
            std::fs::read(dest.path().join("EFI/BOOT/BOOTX64.EFI")).expect("boot file must exist"),
            boot_bytes
        );
        assert_eq!(
            std::fs::read(dest.path().join("overlays/rpi/config.txt"))
                .expect("config file must exist"),
            b"arm_64bit=1"
        );
    }

    #[test]
    fn public_api_format_then_build_outputs_readable_fat_images() {
        // ARRANGE
        let spec = EspSpec::with_uki(Arch::Aarch64, b"uki".to_vec(), vec![]);
        let mut device = Cursor::new(vec![0_u8; 1024 * 1024]);

        // ACT
        esp::format(&mut device).expect("format must succeed");
        let image = esp::build(&spec).expect("build must succeed");

        // ASSERT
        let fs =
            FileSystem::new(&mut device, FsOptions::new()).expect("formatted device must open");
        assert_eq!(fs.volume_label_as_bytes(), b"EFI");

        let mut image_cursor = Cursor::new(image);
        let image_fs =
            FileSystem::new(&mut image_cursor, FsOptions::new()).expect("image must open as FAT");
        let mut boot = image_fs
            .root_dir()
            .open_dir("EFI")
            .expect("EFI dir must exist")
            .open_dir("BOOT")
            .expect("BOOT dir must exist")
            .open_file("BOOTAA64.EFI")
            .expect("boot file must exist");
        let mut boot_bytes = Vec::new();
        std::io::Read::read_to_end(&mut boot, &mut boot_bytes).expect("boot file must read");
        assert_eq!(boot_bytes, b"uki");
    }

    #[test]
    fn public_api_collect_tree_rejects_non_utf8_paths() {
        // ARRANGE
        let root = tempfile::tempdir().expect("temp dir must be created");
        let invalid_name = std::ffi::OsString::from_vec(vec![0x66, 0x6f, 0x80, 0x6f]);
        std::fs::write(root.path().join(invalid_name), b"data").expect("file must be written");

        // ACT
        let result = esp::collect_tree(root.path());

        // ASSERT
        assert!(matches!(result, Err(esp::EspError::InvalidPath(_))));
    }

    #[test]
    fn public_api_collect_tree_propagates_recursive_errors() {
        // ARRANGE
        let root = tempfile::tempdir().expect("temp dir must be created");
        std::fs::create_dir_all(root.path().join("nested")).expect("tree must be created");
        std::fs::write(root.path().join("target.txt"), b"data").expect("target must be written");
        std::os::unix::fs::symlink(
            root.path().join("target.txt"),
            root.path().join("nested/link.txt"),
        )
        .expect("symlink must be created");

        // ACT
        let result = esp::collect_tree(root.path());

        // ASSERT
        assert!(matches!(result, Err(esp::EspError::UnsupportedEntry(_))));
    }

    #[test]
    fn public_api_collect_tree_propagates_read_errors() {
        // ARRANGE
        let root = tempfile::tempdir().expect("temp dir must be created");
        let unreadable = root.path().join("secret.bin");
        std::fs::write(&unreadable, b"data").expect("file must be written");
        let mut permissions = std::fs::metadata(&unreadable)
            .expect("metadata must exist")
            .permissions();
        permissions.set_mode(0o000);
        std::fs::set_permissions(&unreadable, permissions).expect("permissions must be updated");

        // ACT
        let result = esp::collect_tree(root.path());

        // ASSERT
        assert!(matches!(result, Err(esp::EspError::Io(_))));
    }

    #[test]
    fn public_api_populate_writes_all_files_to_directory() {
        // ARRANGE
        let spec = EspSpec::with_uki(
            Arch::X86_64,
            b"uki".to_vec(),
            vec![EspFile {
                path: "overlays/rpi/config.txt".to_owned(),
                data: b"arm_64bit=1".to_vec(),
            }],
        );
        let temp_dir = tempfile::tempdir().expect("temp dir must be created");

        // ACT
        esp::populate(&spec, temp_dir.path()).expect("populate must succeed");

        // ASSERT
        assert_eq!(
            std::fs::read(temp_dir.path().join("EFI/BOOT/BOOTX64.EFI"))
                .expect("boot file must exist"),
            b"uki"
        );
        assert_eq!(
            std::fs::read(temp_dir.path().join("overlays/rpi/config.txt"))
                .expect("config file must exist"),
            b"arm_64bit=1"
        );
    }

    #[test]
    fn public_api_populate_rejects_invalid_paths() {
        // ARRANGE
        let spec = EspSpec {
            files: vec![EspFile {
                path: "../escape".to_owned(),
                data: b"x".to_vec(),
            }],
        };
        let temp_dir = tempfile::tempdir().expect("temp dir must be created");

        // ACT
        let result = esp::populate(&spec, temp_dir.path());

        // ASSERT
        assert!(matches!(result, Err(esp::EspError::InvalidPath(_))));
    }

    #[test]
    fn public_api_populate_propagates_directory_creation_errors() {
        // ARRANGE
        let spec = EspSpec {
            files: vec![EspFile {
                path: "nested/file.txt".to_owned(),
                data: b"data".to_vec(),
            }],
        };
        let temp_dir = tempfile::tempdir().expect("temp dir must be created");
        let root_file = temp_dir.path().join("root-file");
        std::fs::write(&root_file, b"occupied").expect("root file must be written");

        // ACT
        let result = esp::populate(&spec, &root_file);

        // ASSERT
        assert!(matches!(result, Err(esp::EspError::Io(_))));
    }

    #[test]
    fn public_api_populate_propagates_write_errors() {
        // ARRANGE
        let spec = EspSpec {
            files: vec![EspFile {
                path: "existing-dir".to_owned(),
                data: b"data".to_vec(),
            }],
        };
        let temp_dir = tempfile::tempdir().expect("temp dir must be created");
        std::fs::create_dir_all(temp_dir.path().join("existing-dir"))
            .expect("directory must exist");

        // ACT
        let result = esp::populate(&spec, temp_dir.path());

        // ASSERT
        assert!(matches!(result, Err(esp::EspError::Io(_))));
    }
}
