use fatfs::builder;
use fatfs::types::FileSource;

struct TestFile {
    path: String,
    size: u64,
    data: Vec<u8>,
    pos: usize,
}

impl TestFile {
    fn new(path: &str, data: &[u8]) -> Self {
        Self {
            path: path.into(),
            size: u64::try_from(data.len()).unwrap_or(0),
            data: data.to_vec(),
            pos: 0,
        }
    }
}

impl FileSource for TestFile {
    fn path(&self) -> &str {
        &self.path
    }

    fn size(&self) -> u64 {
        self.size
    }

    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let remaining = self.data.len().wrapping_sub(self.pos);
        let to_read = buf.len().min(remaining);
        if let Some(data) = self.data.get(self.pos..self.pos.wrapping_add(to_read))
            && let Some(dst) = buf.get_mut(..to_read)
        {
            dst.copy_from_slice(data);
        }
        self.pos = self.pos.wrapping_add(to_read);
        Ok(to_read)
    }
}

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
        let mut files = vec![TestFile::new("EFI/BOOT/BOOTX64.EFI", b"uki-payload")];

        // ACT
        let mut buf = Vec::new();
        builder::build(&mut files, 1024 * 1024, &mut buf).expect("build must succeed");

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
        let mut files = vec![
            TestFile::new("EFI/BOOT/BOOTX64.EFI", b"uki"),
            TestFile::new("overlays/rpi/config.txt", b"arm_64bit=1"),
        ];

        // ACT
        let mut buf = Vec::new();
        builder::build(&mut files, 1024 * 1024, &mut buf).expect("build must succeed");

        // ASSERT
        assert!(!buf.is_empty());
    }
}
