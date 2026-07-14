#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::os::unix::fs::PermissionsExt as _;

    use esp::FileMeta;
    use esp::builder::{Builder, compute_layout};
    use esp::error::EspError;
    use esp::populate;
    use fatfs::builder;

    #[test]
    fn public_api_build_and_populate_match() {
        // ARRANGE
        let dest = tempfile::tempdir().expect("temp dir must be created");
        let uki_data = b"uki-payload";
        let config_data = b"arm_64bit=1";
        let files = &[
            FileMeta::new(
                "EFI/BOOT/BOOTX64.EFI",
                u64::try_from(uki_data.len()).unwrap_or(0),
            ),
            FileMeta::new(
                "overlays/rpi/config.txt",
                u64::try_from(config_data.len()).unwrap_or(0),
            ),
        ];

        // ACT
        let layout = compute_layout(files).expect("layout must compute");
        let mut buf = Vec::new();
        let mut builder = Builder::new(layout, &mut buf);
        let mut uki_reader = Cursor::new(uki_data.as_slice());
        let mut config_reader = Cursor::new(config_data.as_slice());
        builder
            .add_file(
                "EFI/BOOT/BOOTX64.EFI",
                &mut uki_reader,
                u64::try_from(uki_data.len()).unwrap_or(0),
            )
            .expect("boot file must be added");
        builder
            .add_file(
                "overlays/rpi/config.txt",
                &mut config_reader,
                u64::try_from(config_data.len()).unwrap_or(0),
            )
            .expect("config file must be added");
        builder.finish().expect("build must succeed");

        let mut uki_reader2 = Cursor::new(uki_data.as_slice());
        let mut config_reader2 = Cursor::new(config_data.as_slice());
        let mut readers: Vec<&mut dyn std::io::Read> = vec![&mut uki_reader2, &mut config_reader2];
        populate::write(files, &mut readers, dest.path()).expect("populate must succeed");

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
        let uki_data = b"uki";
        let files = &[FileMeta::new(
            "EFI/BOOT/BOOTAA64.EFI",
            u64::try_from(uki_data.len()).unwrap_or(0),
        )];
        let device_size = 1024_u64 * 1024;
        let mut device = Cursor::new(vec![0_u8; usize::try_from(device_size).unwrap_or(0)]);

        // ACT
        builder::format(&mut device, device_size).expect("format must succeed");
        let device_data = device.into_inner();

        let layout = compute_layout(files).expect("layout must compute");
        let mut buf = Vec::new();
        let mut builder = Builder::new(layout, &mut buf);
        let mut uki_reader = Cursor::new(uki_data.as_slice());
        builder
            .add_file(
                "EFI/BOOT/BOOTAA64.EFI",
                &mut uki_reader,
                u64::try_from(uki_data.len()).unwrap_or(0),
            )
            .expect("boot file must be added");
        builder.finish().expect("build must succeed");

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
        let uki_data = b"uki";
        let files = &[FileMeta::new(
            "EFI/BOOT/BOOTX64.EFI",
            u64::try_from(uki_data.len()).unwrap_or(0),
        )];
        let mut uki_reader = Cursor::new(uki_data.as_slice());
        let mut readers: Vec<&mut dyn std::io::Read> = vec![&mut uki_reader];
        let temp_dir = tempfile::tempdir().expect("temp dir must be created");
        let config_path = temp_dir.path().join("overlays").join("rpi");
        std::fs::create_dir_all(&config_path).expect("dirs must be created");
        std::fs::write(config_path.join("config.txt"), b"x").expect("file must be written");
        std::fs::set_permissions(temp_dir.path(), std::fs::Permissions::from_mode(0o000))
            .expect("permissions must be set");

        // ACT
        let result = populate::write(files, &mut readers, temp_dir.path());

        // ASSERT
        assert!(matches!(result, Err(EspError::Io(_))));
    }

    #[test]
    fn public_api_populate_propagates_directory_creation_errors() {
        // ARRANGE
        let root_file = tempfile::NamedTempFile::new().expect("temp file must be created");
        let uki_data = b"uki";
        let files = &[FileMeta::new(
            "EFI/BOOT/BOOTX64.EFI",
            u64::try_from(uki_data.len()).unwrap_or(0),
        )];
        let mut uki_reader = Cursor::new(uki_data.as_slice());
        let mut readers: Vec<&mut dyn std::io::Read> = vec![&mut uki_reader];

        // ACT
        let result = populate::write(files, &mut readers, root_file.path());

        // ASSERT
        assert!(matches!(result, Err(EspError::Io(_))));
    }
}
