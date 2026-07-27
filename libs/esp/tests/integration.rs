#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use esp::FileMeta;
    use esp::image;
    use esp::layout::compute;
    use fatfs::builder;

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
        assert!(!buf.is_empty());
        assert_eq!(buf.get(510..512), Some(&[0x55, 0xAA][..]), "boot signature");
        assert_eq!(
            device_data.get(71..82),
            Some(b"EFI        ".as_slice()),
            "volume label must be 'EFI' at offset 71"
        );
    }
}
