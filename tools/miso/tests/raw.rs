//! Integration tests for raw GPT disk image building.

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use esp::{Arch, EspFile, EspSpec};
    use miso::error::MisoError;
    use parttable::{
        gpt::{
            table::Table,
            types::{ALIGN_1_MIB_SECTORS, EFI_GUID},
        },
        mbr::types::MBR_PROTECTIVE_GPT_TYPE,
    };

    fn fake_uki(size: usize) -> Vec<u8> {
        let mut uki = Vec::with_capacity(size);
        uki.extend_from_slice(b"MZ");
        uki.resize(size, 0xCC);
        uki
    }

    fn byte_at(data: &[u8], offset: usize) -> u8 {
        data.get(offset).copied().expect("byte must exist")
    }

    fn build_raw(size: usize) -> Vec<u8> {
        let uki_data = fake_uki(size);
        let uki_size = u64::try_from(uki_data.len()).unwrap_or(u64::MAX);
        let boot = EspFile::boot(Arch::X86_64, Cursor::new(uki_data), uki_size);
        let mut spec = EspSpec::builder()
            .add_file(boot)
            .expect("add boot")
            .build()
            .expect("build spec");
        let mut out = Cursor::new(Vec::new());
        miso::build_raw(&mut spec, &mut out, None).expect("build_raw must succeed");
        out.into_inner()
    }

    #[test]
    fn has_valid_gpt() {
        // ARRANGE / ACT
        let img = build_raw(1024);

        // ASSERT
        let mut cursor = Cursor::new(img);
        let gpt = Table::read(&mut cursor).expect("image must contain a valid GPT");
        assert!(
            gpt.has_used_partitions(),
            "GPT must have at least one partition"
        );
    }

    #[test]
    fn has_protective_mbr() {
        // ARRANGE / ACT
        let img = build_raw(512);

        // ASSERT
        assert_eq!(byte_at(&img, 510), 0x55);
        assert_eq!(byte_at(&img, 511), 0xAA);
        assert_eq!(byte_at(&img, 450), MBR_PROTECTIVE_GPT_TYPE);
    }

    #[test]
    fn esp_partition_has_efi_guid() {
        // ARRANGE / ACT
        let img = build_raw(1024);

        // ASSERT
        let mut cursor = Cursor::new(img);
        let gpt = Table::read(&mut cursor).expect("valid GPT");
        let part = gpt.partition(1).expect("must have partition");
        assert_eq!(part.type_guid, EFI_GUID);
    }

    #[test]
    fn disk_size_is_sector_aligned() {
        // ARRANGE / ACT
        let img = build_raw(4096);

        // ASSERT
        assert_eq!(
            img.len().rem_euclid(512),
            0,
            "disk image size must be sector-aligned"
        );
    }

    #[test]
    fn partition_name_is_efi() {
        // ARRANGE / ACT
        let img = build_raw(512);

        // ASSERT
        let mut cursor = Cursor::new(img);
        let gpt = Table::read(&mut cursor).expect("valid GPT");
        let part = gpt.partition(1).expect("must have partition");
        assert_eq!(part.name.as_str(), "EFI");
    }

    #[test]
    fn x86_64_produces_valid_gpt() {
        // ARRANGE / ACT
        let img = build_raw(1024);

        // ASSERT
        let mut cursor = Cursor::new(img);
        let gpt = Table::read(&mut cursor).expect("valid GPT");
        assert!(gpt.has_used_partitions());
    }

    #[test]
    fn compressed_raw_round_trips_to_valid_gpt() {
        // ARRANGE
        let uki_data = fake_uki(1024);
        let uki_size = u64::try_from(uki_data.len()).unwrap_or(u64::MAX);
        let boot = EspFile::boot(Arch::X86_64, Cursor::new(uki_data), uki_size);
        let mut spec = EspSpec::builder()
            .add_file(boot)
            .expect("add boot")
            .build()
            .expect("build spec");

        // ACT
        let mut out = Cursor::new(Vec::new());
        miso::build_raw(&mut spec, &mut out, Some(3)).expect("compressed build_raw must succeed");
        let compressed = out.into_inner();
        let raw = zstd::decode_all(&*compressed).expect("decode compressed raw");

        // ASSERT
        let mut cursor = Cursor::new(raw);
        let gpt = Table::read(&mut cursor).expect("valid GPT");
        assert!(gpt.has_used_partitions());
    }

    #[test]
    fn large_esp_content_grows_the_disk_image() {
        // ARRANGE
        let uki_data = fake_uki(1024);
        let uki_size = u64::try_from(uki_data.len()).unwrap_or(u64::MAX);
        let boot = EspFile::boot(Arch::X86_64, Cursor::new(uki_data), uki_size);

        let extra_data = vec![0x5A_u8; 2 * 1024 * 1024];
        let extra_size = u64::try_from(extra_data.len()).unwrap_or(u64::MAX);
        let extra = EspFile {
            path: "assets/rootfs.img".to_owned(),
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
        miso::build_raw(&mut spec, &mut out, None).expect("build_raw must succeed");
        let img = out.into_inner();

        // ASSERT
        let mut cursor = Cursor::new(&img);
        let gpt = Table::read(&mut cursor).expect("image must contain a valid GPT");
        let part = gpt.partition(1).expect("must have partition");
        let one_mib = usize::try_from(ALIGN_1_MIB_SECTORS * 512).expect("1 MiB must fit in usize");

        assert_eq!(part.type_guid, EFI_GUID);
        assert_eq!(part.name.as_str(), "EFI");
        assert_eq!(part.starting_lba.rem_euclid(ALIGN_1_MIB_SECTORS), 0);
        assert_eq!(
            img.len().rem_euclid(one_mib),
            0,
            "raw disk size must grow in 1 MiB steps"
        );
        assert!(
            img.len() > one_mib * 2,
            "large ESP content must force the raw disk beyond the 2 MiB minimum"
        );
    }

    #[test]
    fn rejects_invalid_compression_level() {
        // ARRANGE
        let uki_data = fake_uki(1024);
        let uki_size = u64::try_from(uki_data.len()).unwrap_or(u64::MAX);
        let boot = EspFile::boot(Arch::X86_64, Cursor::new(uki_data), uki_size);
        let mut spec = EspSpec::builder()
            .add_file(boot)
            .expect("add boot")
            .build()
            .expect("build spec");
        let mut out = Cursor::new(Vec::new());

        // ACT
        let result = miso::build_raw(&mut spec, &mut out, Some(i32::MAX));

        // ASSERT
        assert!(matches!(
            result,
            Err(MisoError::InvalidCompressionLevel { .. })
        ));
    }
}
