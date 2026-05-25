//! Integration tests for miso ISO and RAW building.

use std::io::Cursor;

use esp::{Arch, EspFile, EspSpec};
use miso::iso::SECTOR_SIZE;
use parttable::{
    gpt::{
        table::Table,
        types::{ALIGN_1_MIB_SECTORS, EFI_GUID},
    },
    mbr::types::MBR_PROTECTIVE_GPT_TYPE,
};

fn fake_uki(size: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(size);
    v.extend_from_slice(b"MZ");
    v.resize(size, 0xCC);
    v
}

fn iso_spec(uki: Vec<u8>, arch: Arch) -> EspSpec {
    EspSpec::with_uki(arch, uki, vec![])
}

fn img_spec(uki: Vec<u8>, arch: Arch, files: Vec<EspFile>) -> EspSpec {
    EspSpec::with_uki(arch, uki, files)
}

fn build_iso_bytes(spec: &EspSpec) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    miso::build_iso(spec, &mut out).expect("build_iso must succeed");
    out.into_inner()
}

fn build_raw_bytes(spec: &EspSpec) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    miso::build_raw(spec, &mut out, None).expect("build_raw must succeed");
    out.into_inner()
}

fn build_compressed_raw_bytes(spec: &EspSpec, compression_level: i32) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    miso::build_raw(spec, &mut out, Some(compression_level))
        .expect("compressed build_raw must succeed");
    out.into_inner()
}

#[test]
fn build_iso_cd001_magic_at_sector_16() {
    // ARRANGE
    let spec = iso_spec(fake_uki(4096), Arch::X86_64);

    // ACT
    let iso = build_iso_bytes(&spec);

    // ASSERT
    let offset = SECTOR_SIZE * 16 + 1;
    assert_eq!(
        &iso[offset..offset + 5],
        b"CD001",
        "ISO 9660 magic must appear at byte 32769"
    );
}

#[test]
fn build_iso_pvd_type_is_one() {
    // ARRANGE
    let spec = iso_spec(fake_uki(1024), Arch::X86_64);

    // ACT
    let iso = build_iso_bytes(&spec);

    // ASSERT
    assert_eq!(iso[SECTOR_SIZE * 16], 1, "PVD type byte must be 1");
}

#[test]
fn build_iso_boot_record_vd_type_is_zero() {
    // ARRANGE
    let spec = iso_spec(fake_uki(1024), Arch::X86_64);

    // ACT
    let iso = build_iso_bytes(&spec);

    // ASSERT
    assert_eq!(
        iso[SECTOR_SIZE * 17],
        0,
        "Boot Record VD type byte must be 0"
    );
}

#[test]
fn build_iso_vd_terminator_type_is_255() {
    // ARRANGE
    let spec = iso_spec(fake_uki(1024), Arch::X86_64);

    // ACT
    let iso = build_iso_bytes(&spec);

    // ASSERT
    assert_eq!(
        iso[SECTOR_SIZE * 18],
        255,
        "VD terminator type byte must be 255"
    );
}

#[test]
fn build_iso_output_is_sector_aligned() {
    // ARRANGE
    let spec = iso_spec(fake_uki(3000), Arch::X86_64);

    // ACT
    let iso = build_iso_bytes(&spec);

    // ASSERT
    assert_eq!(
        iso.len() % SECTOR_SIZE,
        0,
        "ISO size must be a multiple of the sector size"
    );
}

#[test]
fn build_iso_mbr_boot_signature_present() {
    // ARRANGE
    let spec = iso_spec(fake_uki(512), Arch::X86_64);

    // ACT
    let iso = build_iso_bytes(&spec);

    // ASSERT
    assert_eq!(iso[510], 0x55, "MBR byte 510 must be 0x55");
    assert_eq!(iso[511], 0xAA, "MBR byte 511 must be 0xAA");
}

#[test]
fn build_iso_mbr_partition_type_is_efi() {
    // ARRANGE
    let spec = iso_spec(fake_uki(512), Arch::X86_64);

    // ACT
    let iso = build_iso_bytes(&spec);

    // ASSERT
    assert_eq!(iso[450], 0xEF, "MBR partition type must be 0xEF (EFI)");
}

#[test]
fn build_iso_aarch64_produces_valid_structure() {
    // ARRANGE
    let spec = iso_spec(fake_uki(1024), Arch::Aarch64);

    // ACT
    let iso = build_iso_bytes(&spec);

    // ASSERT
    let offset = SECTOR_SIZE * 16 + 1;
    assert_eq!(&iso[offset..offset + 5], b"CD001");
}

#[test]
fn build_iso_with_large_uki() {
    // ARRANGE
    let spec = iso_spec(fake_uki(16 * 1024 * 1024), Arch::X86_64);

    // ACT
    let iso = build_iso_bytes(&spec);

    // ASSERT
    let offset = SECTOR_SIZE * 16 + 1;
    assert_eq!(&iso[offset..offset + 5], b"CD001");
    assert!(
        iso.len() > 16 * 1024 * 1024,
        "ISO must be larger than the UKI"
    );
}

#[test]
fn build_iso_boot_catalog_validation_checksum_valid() {
    // ARRANGE
    let spec = iso_spec(fake_uki(512), Arch::X86_64);

    // ACT
    let iso = build_iso_bytes(&spec);

    // ASSERT
    let cat_start = SECTOR_SIZE * 21;
    let validation = &iso[cat_start..cat_start + 32];
    let sum: u32 = validation
        .chunks(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]) as u32)
        .sum();
    assert_eq!(
        sum % 0x10000,
        0,
        "El Torito validation entry checksum must sum to zero mod 0x10000"
    );
}

#[test]
fn build_iso_boot_catalog_has_55aa_keys() {
    // ARRANGE
    let spec = iso_spec(fake_uki(512), Arch::X86_64);

    // ACT
    let iso = build_iso_bytes(&spec);

    // ASSERT
    let cat_start = SECTOR_SIZE * 21;
    assert_eq!(
        iso[cat_start + 30],
        0x55,
        "El Torito key byte 1 must be 0x55"
    );
    assert_eq!(
        iso[cat_start + 31],
        0xAA,
        "El Torito key byte 2 must be 0xAA"
    );
}

#[test]
fn build_iso_el_torito_platform_id_is_efi() {
    // ARRANGE
    let spec = iso_spec(fake_uki(512), Arch::X86_64);

    // ACT
    let iso = build_iso_bytes(&spec);

    // ASSERT
    let cat_start = SECTOR_SIZE * 21;
    assert_eq!(
        iso[cat_start + 1],
        0xEF,
        "El Torito platform ID must be 0xEF (EFI)"
    );
}

#[test]
fn build_iso_default_entry_is_bootable() {
    // ARRANGE
    let spec = iso_spec(fake_uki(512), Arch::X86_64);

    // ACT
    let iso = build_iso_bytes(&spec);

    // ASSERT
    let cat_start = SECTOR_SIZE * 21;
    assert_eq!(
        iso[cat_start + 32],
        0x88,
        "El Torito default entry boot indicator must be 0x88 (bootable)"
    );
}

#[test]
fn build_iso_with_extra_files_includes_recursive_dirs() {
    // ARRANGE
    let spec = EspSpec::with_uki(
        Arch::X86_64,
        fake_uki(512),
        vec![EspFile {
            path: "overlays/rpi/config.txt".to_owned(),
            data: b"arm_64bit=1".to_vec(),
        }],
    );

    // ACT
    let iso = build_iso_bytes(&spec);

    // ASSERT
    let offset = SECTOR_SIZE * 16 + 1;
    assert_eq!(&iso[offset..offset + 5], b"CD001");
}

#[test]
fn build_raw_has_valid_gpt() {
    // ARRANGE
    let spec = img_spec(fake_uki(1024), Arch::Aarch64, vec![]);

    // ACT
    let img = build_raw_bytes(&spec);

    // ASSERT
    let mut cursor = Cursor::new(img);
    let gpt = Table::read(&mut cursor).expect("image must contain a valid GPT");
    assert!(
        gpt.has_used_partitions(),
        "GPT must have at least one partition"
    );
}

#[test]
fn build_raw_has_protective_mbr() {
    // ARRANGE
    let spec = img_spec(fake_uki(512), Arch::Aarch64, vec![]);

    // ACT
    let img = build_raw_bytes(&spec);

    // ASSERT
    assert_eq!(img[510], 0x55, "MBR byte 510 must be 0x55");
    assert_eq!(img[511], 0xAA, "MBR byte 511 must be 0xAA");
    assert_eq!(
        img[450], MBR_PROTECTIVE_GPT_TYPE,
        "MBR partition type must be 0xEE (GPT protective)"
    );
}

#[test]
fn build_raw_esp_has_efi_system_partition_guid() {
    // ARRANGE
    let spec = img_spec(fake_uki(1024), Arch::Aarch64, vec![]);

    // ACT
    let img = build_raw_bytes(&spec);

    // ASSERT
    let mut cursor = Cursor::new(img);
    let gpt = Table::read(&mut cursor).expect("valid GPT");
    let part = gpt.partition(1).expect("must have partition");
    assert_eq!(part.type_guid, EFI_GUID);
}

#[test]
fn build_raw_disk_size_is_sector_aligned() {
    // ARRANGE
    let spec = img_spec(fake_uki(4096), Arch::Aarch64, vec![]);

    // ACT
    let img = build_raw_bytes(&spec);

    // ASSERT
    assert_eq!(img.len() % 512, 0, "disk image size must be sector-aligned");
}

#[test]
fn build_raw_partition_name_is_efi() {
    // ARRANGE
    let spec = img_spec(fake_uki(512), Arch::Aarch64, vec![]);

    // ACT
    let img = build_raw_bytes(&spec);

    // ASSERT
    let mut cursor = Cursor::new(img);
    let gpt = Table::read(&mut cursor).expect("valid GPT");
    let part = gpt.partition(1).expect("must have partition");
    assert_eq!(part.name.as_str(), "EFI");
}

#[test]
fn build_raw_x86_64_produces_valid_gpt() {
    // ARRANGE
    let spec = img_spec(fake_uki(1024), Arch::X86_64, vec![]);

    // ACT
    let img = build_raw_bytes(&spec);

    // ASSERT
    let mut cursor = Cursor::new(img);
    let gpt = Table::read(&mut cursor).expect("valid GPT");
    assert!(gpt.has_used_partitions());
}

#[test]
fn build_compressed_raw_round_trips_to_valid_gpt() {
    // ARRANGE
    let spec = img_spec(fake_uki(1024), Arch::Aarch64, vec![]);

    // ACT
    let compressed = build_compressed_raw_bytes(&spec, 3);
    let raw = zstd::decode_all(&compressed[..]).expect("decode compressed raw");

    // ASSERT
    let mut cursor = Cursor::new(raw);
    let gpt = Table::read(&mut cursor).expect("valid GPT");
    assert!(gpt.has_used_partitions());
}

#[test]
fn build_raw_large_esp_content_grows_the_disk_image() {
    // ARRANGE
    let spec = img_spec(
        fake_uki(1024),
        Arch::Aarch64,
        vec![EspFile {
            path: "assets/rootfs.img".to_owned(),
            data: vec![0x5Au8; 2 * 1024 * 1024],
        }],
    );

    // ACT
    let img = build_raw_bytes(&spec);

    // ASSERT
    let mut cursor = Cursor::new(&img);
    let gpt = Table::read(&mut cursor).expect("image must contain a valid GPT");
    let part = gpt.partition(1).expect("must have partition");
    let one_mib = (ALIGN_1_MIB_SECTORS * 512) as usize;

    assert_eq!(part.type_guid, EFI_GUID);
    assert_eq!(part.name.as_str(), "EFI");
    assert_eq!(part.starting_lba % ALIGN_1_MIB_SECTORS, 0);
    assert_eq!(
        img.len() % one_mib,
        0,
        "raw disk size must grow in 1 MiB steps"
    );
    assert!(
        img.len() > one_mib * 2,
        "large ESP content must force the raw disk beyond the 2 MiB minimum"
    );
}
