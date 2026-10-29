#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use esp::layout::compute;
    use esp::{FileMeta, image};
    use fatfs::builder;

    const SECTOR_SIZE: usize = 512;
    const VOLUME_LABEL: &[u8] = b"MUAKOS     ";

    #[test]
    fn public_api_build_and_builder_format_match() {
        // ARRANGE
        let uki_data = vec![0xAB_u8; 33 * 1024 * 1024];
        let files = &[FileMeta::new(
            "EFI/BOOT/BOOTAA64.EFI",
            u64::try_from(uki_data.len()).unwrap_or(0),
        )];
        let device_size = 36_u64 * 1024 * 1024;
        let mut device = Cursor::new(vec![0_u8; usize::try_from(device_size).unwrap_or(0)]);

        // ACT
        builder::format(&mut device, device_size).expect("format must succeed");
        let device_data = device.into_inner();

        let layout = compute(files).expect("layout must compute");
        let mut buf = Vec::new();
        let mut uki_reader = Cursor::new(uki_data.as_slice());
        let mut readers: Vec<&mut dyn std::io::Read> = vec![&mut uki_reader];
        image::build(&layout, &mut readers, &mut buf).expect("image::build must succeed");

        // ASSERT
        assert_ne!(buf, b"");
        assert_eq!(buf.get(510..512), Some(&[0x55, 0xAA][..]), "boot signature");
        assert_eq!(
            device_data.get(71..82),
            Some(VOLUME_LABEL),
            "volume label must be 'MUAKOS' at offset 71"
        );
    }

    #[test]
    fn esp_image_carries_a_backup_boot_sector() {
        // ARRANGE
        let files = &[FileMeta::new("EFI/BOOT/BOOTAA64.EFI", 4096)];
        let layout = compute(files).expect("layout must compute");
        let mut buf = Vec::new();
        let mut reader = Cursor::new(vec![0xAB_u8; 4096]);
        let mut readers: Vec<&mut dyn std::io::Read> = vec![&mut reader];

        // ACT
        image::build(&layout, &mut readers, &mut buf).expect("image::build must succeed");

        // ASSERT
        let backup = SECTOR_SIZE * 6;
        assert_eq!(
            buf.get(..SECTOR_SIZE),
            buf.get(backup..backup + SECTOR_SIZE),
            "the backup boot sector must copy sector 0"
        );
    }

    #[test]
    fn esp_image_keeps_free_space() {
        // ARRANGE
        let payload = 69 * 1024 * 1024;
        let files = &[FileMeta::new("EFI/BOOT/BOOTAA64.EFI", payload)];

        // ACT
        let layout = compute(files).expect("layout must compute");

        // ASSERT
        assert!(
            layout.total_size >= payload + fatfs::builder::FREE_SLACK,
            "the ESP must keep room for runtime writes"
        );
    }
}
