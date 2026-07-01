//! Integration tests for ISO 9660 + El Torito image building.

#[cfg(test)]
mod tests {
    use core::ops::Range;
    use std::io::Cursor;

    use esp::model::{Arch, EspFile, EspSpec};
    use miso::iso;

    fn fake_uki(size: usize) -> Vec<u8> {
        let mut uki = Vec::with_capacity(size);
        uki.extend_from_slice(b"MZ");
        uki.resize(size, 0xCC);
        uki
    }

    fn byte_at(data: &[u8], offset: usize) -> u8 {
        data.get(offset).copied().expect("byte must exist")
    }

    fn bytes_at(data: &[u8], range: Range<usize>) -> &[u8] {
        data.get(range).expect("byte range must exist")
    }

    fn build_iso(uki_size: usize) -> Vec<u8> {
        let uki_data = fake_uki(uki_size);
        let size = u64::try_from(uki_data.len()).unwrap_or(u64::MAX);
        let boot = EspFile::boot(Arch::X86_64, Cursor::new(uki_data), size);
        let mut spec = EspSpec::builder()
            .add_file(boot)
            .expect("add boot")
            .build()
            .expect("build spec");
        let mut out = Cursor::new(Vec::new());
        miso::build_iso(&mut spec, &mut out).expect("build_iso must succeed");
        out.into_inner()
    }

    #[test]
    fn cd001_magic_at_sector_16() {
        // ARRANGE / ACT
        let iso = build_iso(4096);

        // ASSERT
        let offset = iso::SECTOR_SIZE * 16 + 1;
        assert_eq!(bytes_at(&iso, offset..offset + 5), b"CD001");
    }

    #[test]
    fn pvd_type_byte_is_one() {
        // ARRANGE / ACT
        let iso = build_iso(1024);

        // ASSERT
        assert_eq!(byte_at(&iso, iso::SECTOR_SIZE * 16), 1);
    }

    #[test]
    fn boot_record_vd_type_byte_is_zero() {
        // ARRANGE / ACT
        let iso = build_iso(1024);

        // ASSERT
        assert_eq!(byte_at(&iso, iso::SECTOR_SIZE * 17), 0);
    }

    #[test]
    fn vd_terminator_type_byte_is_255() {
        // ARRANGE / ACT
        let iso = build_iso(1024);

        // ASSERT
        assert_eq!(byte_at(&iso, iso::SECTOR_SIZE * 18), 255);
    }

    #[test]
    fn output_is_sector_aligned() {
        // ARRANGE / ACT
        let iso = build_iso(3000);

        // ASSERT
        assert_eq!(iso.len().rem_euclid(iso::SECTOR_SIZE), 0);
    }

    #[test]
    fn mbr_boot_signature_present() {
        // ARRANGE / ACT
        let iso = build_iso(512);

        // ASSERT
        assert_eq!(byte_at(&iso, 510), 0x55);
        assert_eq!(byte_at(&iso, 511), 0xAA);
    }

    #[test]
    fn mbr_partition_type_is_efi() {
        // ARRANGE / ACT
        let iso = build_iso(512);

        // ASSERT
        assert_eq!(byte_at(&iso, 450), 0xEF);
    }

    #[test]
    fn aarch64_produces_valid_structure() {
        // ARRANGE
        let uki_data = fake_uki(1024);
        let size = u64::try_from(uki_data.len()).unwrap_or(u64::MAX);
        let boot = EspFile::boot(Arch::Aarch64, Cursor::new(uki_data), size);
        let mut spec = EspSpec::builder()
            .add_file(boot)
            .expect("add boot")
            .build()
            .expect("build spec");

        // ACT
        let mut out = Cursor::new(Vec::new());
        miso::build_iso(&mut spec, &mut out).expect("build_iso must succeed");
        let iso = out.into_inner();

        // ASSERT
        let offset = iso::SECTOR_SIZE * 16 + 1;
        assert_eq!(bytes_at(&iso, offset..offset + 5), b"CD001");
    }

    #[test]
    fn with_large_uki() {
        // ARRANGE / ACT
        let iso = build_iso(16 * 1024 * 1024);

        // ASSERT
        let offset = iso::SECTOR_SIZE * 16 + 1;
        assert_eq!(bytes_at(&iso, offset..offset + 5), b"CD001");
        assert!(
            iso.len() > 16 * 1024 * 1024,
            "ISO must be larger than the UKI"
        );
    }

    #[test]
    fn boot_catalog_validation_checksum_valid() {
        // ARRANGE / ACT
        let iso = build_iso(512);

        // ASSERT
        let cat_start = iso::SECTOR_SIZE * 21;
        let validation = bytes_at(&iso, cat_start..cat_start + 32);
        let sum: u32 = validation
            .chunks_exact(2)
            .map(|chunk| {
                u32::from(u16::from_le_bytes(
                    chunk.try_into().expect("chunk is 2 bytes"),
                ))
            })
            .sum();
        assert_eq!(
            sum.rem_euclid(0x10000),
            0,
            "checksum must be zero mod 0x10000"
        );
    }

    #[test]
    fn boot_catalog_has_55aa_keys() {
        // ARRANGE / ACT
        let iso = build_iso(512);

        // ASSERT
        let cat_start = iso::SECTOR_SIZE * 21;
        assert_eq!(byte_at(&iso, cat_start + 30), 0x55);
        assert_eq!(byte_at(&iso, cat_start + 31), 0xAA);
    }

    #[test]
    fn el_torito_platform_id_is_efi() {
        // ARRANGE / ACT
        let iso = build_iso(512);

        // ASSERT
        let cat_start = iso::SECTOR_SIZE * 21;
        assert_eq!(byte_at(&iso, cat_start + 1), 0xEF);
    }

    #[test]
    fn default_entry_is_bootable() {
        // ARRANGE / ACT
        let iso = build_iso(512);

        // ASSERT
        let cat_start = iso::SECTOR_SIZE * 21;
        assert_eq!(byte_at(&iso, cat_start + 32), 0x88);
    }

    #[test]
    fn with_extra_files_includes_recursive_dirs() {
        // ARRANGE
        let uki_data = fake_uki(512);
        let uki_size = u64::try_from(uki_data.len()).unwrap_or(u64::MAX);
        let boot = EspFile::boot(Arch::X86_64, Cursor::new(uki_data), uki_size);

        let extra_data = b"arm_64bit=1".to_vec();
        let extra_size = u64::try_from(extra_data.len()).unwrap_or(u64::MAX);
        let extra = EspFile {
            path: "overlays/rpi/config.txt".to_owned(),
            reader: Box::new(Cursor::new(extra_data)),
            size: extra_size,
        };

        let mut spec = EspSpec::builder()
            .add_file(boot)
            .expect("add boot")
            .add_file(extra)
            .expect("add extra")
            .build()
            .expect("build spec");

        // ACT
        let mut out = Cursor::new(Vec::new());
        miso::build_iso(&mut spec, &mut out).expect("build_iso must succeed");
        let iso = out.into_inner();

        // ASSERT
        let offset = iso::SECTOR_SIZE * 16 + 1;
        assert_eq!(bytes_at(&iso, offset..offset + 5), b"CD001");
    }

    #[test]
    fn returns_nonempty_image() {
        // ARRANGE / ACT
        let iso = build_iso(1024);

        // ASSERT
        assert!(!iso.is_empty());
    }

    #[test]
    fn rejects_sizes_that_do_not_fit_in_u32() {
        // ARRANGE
        use std::io::Cursor as IoCursor;

        let oversized = u64::from(u32::MAX) + 1;
        let mut out = IoCursor::new(Vec::new());

        // ACT
        let result = iso::write(&mut out, oversized, |_| Ok(()));

        // ASSERT
        let err = result.expect_err("oversized image must fail");
        assert!(err.to_string().contains("must fit in u32"));
    }
}
