//! Integration tests for miso ISO building.

use miso::{Arch, SECTOR_SIZE};

fn fake_uki(size: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(size);
    v.extend_from_slice(b"MZ");
    v.resize(size, 0xCC);
    v
}

#[test]
fn build_iso_cd001_magic_at_sector_16() {
    // ARRANGE
    let uki = fake_uki(4096);

    // ACT
    let iso = miso::build_iso(&uki, Arch::X86_64, "MUAK").expect("build_iso must succeed");

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
    let uki = fake_uki(1024);

    // ACT
    let iso = miso::build_iso(&uki, Arch::X86_64, "MUAK").expect("build_iso must succeed");

    // ASSERT
    assert_eq!(iso[SECTOR_SIZE * 16], 1, "PVD type byte must be 1");
}

#[test]
fn build_iso_boot_record_vd_type_is_zero() {
    // ARRANGE
    let uki = fake_uki(1024);

    // ACT
    let iso = miso::build_iso(&uki, Arch::X86_64, "MUAK").expect("build_iso must succeed");

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
    let uki = fake_uki(1024);

    // ACT
    let iso = miso::build_iso(&uki, Arch::X86_64, "MUAK").expect("build_iso must succeed");

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
    let uki = fake_uki(3000);

    // ACT
    let iso = miso::build_iso(&uki, Arch::X86_64, "MUAK").expect("build_iso must succeed");

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
    let uki = fake_uki(512);

    // ACT
    let iso = miso::build_iso(&uki, Arch::X86_64, "MUAK").expect("build_iso must succeed");

    // ASSERT
    assert_eq!(iso[510], 0x55, "MBR byte 510 must be 0x55");
    assert_eq!(iso[511], 0xAA, "MBR byte 511 must be 0xAA");
}

#[test]
fn build_iso_mbr_partition_type_is_efi() {
    // ARRANGE
    let uki = fake_uki(512);

    // ACT
    let iso = miso::build_iso(&uki, Arch::X86_64, "MUAK").expect("build_iso must succeed");

    // ASSERT
    assert_eq!(iso[450], 0xEF, "MBR partition type must be 0xEF (EFI)");
}

#[test]
fn build_iso_volume_label_in_pvd() {
    // ARRANGE
    let uki = fake_uki(512);

    // ACT
    let iso = miso::build_iso(&uki, Arch::X86_64, "MYVOLUME").expect("build_iso must succeed");

    // ASSERT
    let pvd_start = SECTOR_SIZE * 16;
    let label = &iso[pvd_start + 40..pvd_start + 72];
    assert!(
        label.starts_with(b"MYVOLUME"),
        "PVD must contain the volume label, got: {:?}",
        &label[..8]
    );
}

#[test]
fn build_iso_aarch64_produces_valid_structure() {
    // ARRANGE
    let uki = fake_uki(1024);

    // ACT
    let iso =
        miso::build_iso(&uki, Arch::Aarch64, "MUAK").expect("build_iso must succeed for aarch64");

    // ASSERT
    let offset = SECTOR_SIZE * 16 + 1;
    assert_eq!(&iso[offset..offset + 5], b"CD001");
}

#[test]
fn build_iso_with_large_uki() {
    // ARRANGE
    let uki = fake_uki(16 * 1024 * 1024);

    // ACT
    let iso =
        miso::build_iso(&uki, Arch::X86_64, "MUAK").expect("build_iso must succeed for large UKI");

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
    let uki = fake_uki(512);

    // ACT
    let iso = miso::build_iso(&uki, Arch::X86_64, "MUAK").expect("build_iso must succeed");

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
    let uki = fake_uki(512);

    // ACT
    let iso = miso::build_iso(&uki, Arch::X86_64, "MUAK").expect("build_iso must succeed");

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
    let uki = fake_uki(512);

    // ACT
    let iso = miso::build_iso(&uki, Arch::X86_64, "MUAK").expect("build_iso must succeed");

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
    let uki = fake_uki(512);

    // ACT
    let iso = miso::build_iso(&uki, Arch::X86_64, "MUAK").expect("build_iso must succeed");

    // ASSERT
    let cat_start = SECTOR_SIZE * 21;
    assert_eq!(
        iso[cat_start + 32],
        0x88,
        "El Torito default entry boot indicator must be 0x88 (bootable)"
    );
}
