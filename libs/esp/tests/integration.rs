#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::os::unix::fs::PermissionsExt as _;

    use esp::error::EspError;
    use esp::image;
    use esp::model::{Arch, EspFile, EspSpec};
    use esp::populate;
    use fatfs::builder;

    fn boot_file(data: &mut Cursor<Vec<u8>>) -> EspFile<'_> {
        let size = u64::try_from(data.get_ref().len()).unwrap_or(0);

        EspFile::boot(Arch::X86_64, data, size)
    }

    fn aarch64_boot_file(data: &mut Cursor<Vec<u8>>) -> EspFile<'_> {
        let size = u64::try_from(data.get_ref().len()).unwrap_or(0);

        EspFile::boot(Arch::Aarch64, data, size)
    }

    fn overlay_file<'a>(path: &str, cursor: &'a mut Cursor<Vec<u8>>) -> EspFile<'a> {
        let size = u64::try_from(cursor.get_ref().len()).unwrap_or(0);

        EspFile {
            path: path.to_owned(),
            reader: cursor,
            size,
        }
    }

    #[test]
    fn public_api_build_and_populate_match() {
        // ARRANGE
        let dest = tempfile::tempdir().expect("temp dir must be created");
        let mut ukic = Cursor::new(b"uki-payload".to_vec());
        let mut config_cursor = Cursor::new(b"arm_64bit=1".to_vec());
        let mut files = vec![
            boot_file(&mut ukic),
            overlay_file("overlays/rpi/config.txt", &mut config_cursor),
        ];
        let mut ukic2 = Cursor::new(b"uki-payload".to_vec());
        let mut config_cursor2 = Cursor::new(b"arm_64bit=1".to_vec());
        let mut spec = EspSpec::builder()
            .add_file(boot_file(&mut ukic2))
            .expect("boot file must be added")
            .add_file(overlay_file("overlays/rpi/config.txt", &mut config_cursor2))
            .expect("config file must be added")
            .build()
            .expect("spec must build");

        // ACT
        let mut buf = Vec::new();
        image::build(&mut files, &mut buf).expect("build must succeed");
        populate::write(spec.files_mut(), dest.path()).expect("populate must succeed");

        // ASSERT
        assert!(!buf.is_empty());
        assert_eq!(buf.get(510..512), Some(&[0x55, 0xAA][..]), "boot signature");
        assert_eq!(
            std::fs::read(dest.path().join("EFI/BOOT/BOOTX64.EFI")).expect("boot file must exist"),
            b"uki-payload"
        );
        assert_eq!(
            std::fs::read(dest.path().join("overlays/rpi/config.txt"))
                .expect("config file must exist"),
            b"arm_64bit=1"
        );
    }

    #[test]
    fn public_api_format_then_build_outputs_bootable_images() {
        // ARRANGE
        let mut ukic = Cursor::new(b"uki".to_vec());
        let mut files = vec![aarch64_boot_file(&mut ukic)];
        let device_size = 1024_u64 * 1024;
        let mut device = Cursor::new(vec![0_u8; usize::try_from(device_size).unwrap_or(0)]);

        // ACT
        builder::format(&mut device, device_size).expect("format must succeed");
        let device_data = device.into_inner();
        let mut buf = Vec::new();
        image::build(&mut files, &mut buf).expect("build must succeed");

        // ASSERT
        let vol_label_43 = device_data.get(43..54).unwrap_or(&[]);
        let vol_label_71 = device_data.get(71..82).unwrap_or(&[]);
        let has_label = vol_label_43 == b"EFI        " || vol_label_71 == b"EFI        ";
        assert!(
            has_label,
            "volume label must be 'EFI' at offset 43 (FAT12) or 71 (FAT32), got 43:{vol_label_43:?} 71:{vol_label_71:?}"
        );
        assert!(!buf.is_empty());
        assert_eq!(buf.get(510..512), Some(&[0x55, 0xAA][..]), "boot signature");
    }

    #[test]
    fn public_api_populate_propagates_write_errors() {
        // ARRANGE
        let mut ukic = Cursor::new(b"uki".to_vec());
        let mut spec = EspSpec::builder()
            .add_file(boot_file(&mut ukic))
            .expect("boot file must be added")
            .build()
            .expect("spec must build");
        let temp_dir = tempfile::tempdir().expect("temp dir must be created");
        let config_path = temp_dir.path().join("overlays").join("rpi");
        std::fs::create_dir_all(&config_path).expect("dirs must be created");
        std::fs::write(config_path.join("config.txt"), b"x").expect("file must be written");
        std::fs::set_permissions(temp_dir.path(), std::fs::Permissions::from_mode(0o000))
            .expect("permissions must be set");

        // ACT
        let result = populate::write(spec.files_mut(), temp_dir.path());

        // ASSERT
        assert!(matches!(result, Err(EspError::Io(_))));
    }

    #[test]
    fn public_api_populate_propagates_directory_creation_errors() {
        // ARRANGE
        let root_file = tempfile::NamedTempFile::new().expect("temp file must be created");
        let mut ukic = Cursor::new(b"uki".to_vec());
        let mut spec = EspSpec::builder()
            .add_file(boot_file(&mut ukic))
            .expect("boot file must be added")
            .build()
            .expect("spec must build");

        // ACT
        let result = populate::write(spec.files_mut(), root_file.path());

        // ASSERT
        assert!(matches!(result, Err(EspError::Io(_))));
    }
}
