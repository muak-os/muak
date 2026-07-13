use std::io::Cursor;

use fatfs::builder;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_creates_bootable_image() {
        // ARRANGE
        let mut buf = Vec::new();

        // ACT
        builder::format(&mut buf, 1024 * 1024).expect("format must succeed");

        // ASSERT
        assert_eq!(
            buf.get(510..512),
            Some(&[0x55, 0xAA][..]),
            "boot signature must be valid"
        );
        assert_eq!(buf.get(3..11), Some(&b"MSWIN4.1"[..]), "OEM ID must match");
    }

    #[test]
    fn build_creates_image_with_content() {
        // ARRANGE
        let files: &[(&str, u64)] = &[("EFI/BOOT/BOOTX64.EFI", 11)];
        let precomputed = builder::precompute(files, 1024 * 1024).expect("precompute must succeed");
        let mut readers = vec![Cursor::new(b"uki-payload".as_slice())];

        // ACT
        let mut buf = Vec::new();
        builder::build(&precomputed, &mut readers, &mut buf).expect("build must succeed");

        // ASSERT
        assert_eq!(
            buf.get(510..512),
            Some(&[0x55, 0xAA][..]),
            "boot signature must be valid"
        );
        assert!(buf.len() >= 1024 * 1024);
    }

    #[test]
    fn format_rejects_tiny_image() {
        // ARRANGE
        let mut buf = Vec::new();

        // ACT
        let result = builder::format(&mut buf, 100);

        // ASSERT
        assert!(result.is_err(), "tiny image must be rejected");
    }

    #[test]
    fn build_with_nested_paths_succeeds() {
        // ARRANGE
        let files: &[(&str, u64)] = &[("EFI/BOOT/BOOTX64.EFI", 3), ("overlays/rpi/config.txt", 11)];
        let precomputed = builder::precompute(files, 1024 * 1024).expect("precompute must succeed");
        let mut readers = vec![
            Cursor::new(b"uki".as_slice()),
            Cursor::new(b"arm_64bit=1".as_slice()),
        ];

        // ACT
        let mut buf = Vec::new();
        builder::build(&precomputed, &mut readers, &mut buf).expect("build must succeed");

        // ASSERT
        assert!(!buf.is_empty());
    }
}
