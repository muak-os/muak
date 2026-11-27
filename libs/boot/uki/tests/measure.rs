//! Integration tests for streaming UKI section measurement records.

#[cfg(test)]
mod common;

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};

    use sha2::{Digest as _, Sha256};
    use uki::measure::from_file;
    use uki::section::{CMDLINE, INITRD, KERNEL};

    use crate::common::{add_section, build_test_pe};

    fn image_file(sections: &[([u8; 8], &[u8])]) -> tempfile::NamedTempFile {
        let mut data = build_test_pe();
        for &(name, content) in sections {
            add_section(&mut data, name, content);
        }
        let file = tempfile::NamedTempFile::new().expect("temp file");
        file.as_file().write_all(&data).expect("write image");

        file
    }

    #[test]
    fn reads_sections_in_table_order() {
        // ARRANGE
        let file = image_file(&[
            (*b".cmdline", b"console=ttyS0".as_slice()),
            (*b".kernel\0", b"kernel-image".as_slice()),
            (*b".initrd\0", b"initramfs".as_slice()),
        ]);

        // ACT
        let records = from_file(file.path()).expect("measure");

        // ASSERT
        let names: Vec<_> = records.iter().map(|record| record.name).collect();
        assert_eq!(names, [CMDLINE, KERNEL, INITRD]);
    }

    #[test]
    fn section_hashes_cover_the_content_bytes() {
        // ARRANGE
        let file = image_file(&[
            (*b".cmdline", b"quiet".as_slice()),
            (*b".kernel\0", b"kernel-image".as_slice()),
            (*b".initrd\0", b"initramfs".as_slice()),
        ]);

        // ACT
        let records = from_file(file.path()).expect("measure");

        // ASSERT
        for (record, content) in
            records
                .iter()
                .zip([b"quiet".as_slice(), b"kernel-image", b"initramfs"])
        {
            let mut expected = [0; 32];
            expected.copy_from_slice(Sha256::digest(content).as_ref());
            assert_eq!(record.hash, expected, "hash mismatch for {}", record.name);
        }
    }

    #[test]
    fn skips_unknown_sections() {
        // ARRANGE
        let file = image_file(&[
            (*b".text\0\0\0", b"stub-code".as_slice()),
            (*b".kernel\0", b"kernel-image".as_slice()),
            (*b".bss\0\0\0\0", b"stub-data".as_slice()),
        ]);

        // ACT
        let records = from_file(file.path()).expect("measure");

        // ASSERT
        assert_eq!(records.len(), 1);
        assert_eq!(records.first().expect("record").name, KERNEL);
    }

    #[test]
    fn rejects_duplicate_uki_sections() {
        // ARRANGE
        let file = image_file(&[
            (*b".kernel\0", b"first".as_slice()),
            (*b".kernel\0", b"second".as_slice()),
        ]);

        // ACT
        let error = from_file(file.path()).unwrap_err();

        // ASSERT
        assert!(error.to_string().contains("duplicate"), "{error}");
    }

    #[test]
    fn rejects_missing_kernel_section() {
        // ARRANGE
        let file = image_file(&[(*b".cmdline", b"quiet".as_slice())]);

        // ACT
        let error = from_file(file.path()).unwrap_err();

        // ASSERT
        assert!(error.to_string().contains(".kernel"), "{error}");
    }

    #[test]
    fn rejects_truncated_images() {
        // ARRANGE
        let file = image_file(&[(*b".kernel\0", b"kernel-image".as_slice())]);
        let mut data = Vec::new();
        file.as_file().read_to_end(&mut data).expect("read image");
        let truncated = tempfile::NamedTempFile::new().expect("temp file");
        let cut = data.len().saturating_sub(4);
        truncated
            .as_file()
            .write_all(data.get(..cut).expect("image prefix"))
            .expect("write truncated image");

        // ACT
        let error = from_file(truncated.path()).unwrap_err();

        // ASSERT
        assert!(
            error.to_string().contains("truncated") || error.to_string().contains("I/O"),
            "{error}"
        );
    }

    #[test]
    fn rejects_invalid_pe_signature() {
        // ARRANGE
        let mut data = build_test_pe();
        // Corrupt the PE signature at the NT headers offset.
        data.get_mut(0x40..0x42)
            .expect("pe signature range")
            .copy_from_slice(b"XX");
        let file = tempfile::NamedTempFile::new().expect("temp file");
        file.as_file().write_all(&data).expect("write junk");

        // ACT
        let error = from_file(file.path()).unwrap_err();

        // ASSERT
        assert!(error.to_string().contains("PE signature"), "{error}");
    }

    #[test]
    fn rejects_short_files() {
        // ARRANGE
        let file = tempfile::NamedTempFile::new().expect("temp file");
        file.as_file().write_all(b"short").expect("write junk");

        // ACT
        let error = from_file(file.path()).unwrap_err();

        // ASSERT
        assert!(error.to_string().contains("I/O"), "{error}");
    }
}
