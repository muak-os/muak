//! Integration tests for miso ISO and IMG building.

use std::io::Cursor;

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
    let iso = miso::build_iso(&uki, Arch::X86_64).expect("build_iso must succeed");

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
    let iso = miso::build_iso(&uki, Arch::X86_64).expect("build_iso must succeed");

    // ASSERT
    assert_eq!(iso[SECTOR_SIZE * 16], 1, "PVD type byte must be 1");
}

#[test]
fn build_iso_boot_record_vd_type_is_zero() {
    // ARRANGE
    let uki = fake_uki(1024);

    // ACT
    let iso = miso::build_iso(&uki, Arch::X86_64).expect("build_iso must succeed");

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
    let iso = miso::build_iso(&uki, Arch::X86_64).expect("build_iso must succeed");

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
    let iso = miso::build_iso(&uki, Arch::X86_64).expect("build_iso must succeed");

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
    let iso = miso::build_iso(&uki, Arch::X86_64).expect("build_iso must succeed");

    // ASSERT
    assert_eq!(iso[510], 0x55, "MBR byte 510 must be 0x55");
    assert_eq!(iso[511], 0xAA, "MBR byte 511 must be 0xAA");
}

#[test]
fn build_iso_mbr_partition_type_is_efi() {
    // ARRANGE
    let uki = fake_uki(512);

    // ACT
    let iso = miso::build_iso(&uki, Arch::X86_64).expect("build_iso must succeed");

    // ASSERT
    assert_eq!(iso[450], 0xEF, "MBR partition type must be 0xEF (EFI)");
}

#[test]
fn build_iso_aarch64_produces_valid_structure() {
    // ARRANGE
    let uki = fake_uki(1024);

    // ACT
    let iso = miso::build_iso(&uki, Arch::Aarch64).expect("build_iso must succeed for aarch64");

    // ASSERT
    let offset = SECTOR_SIZE * 16 + 1;
    assert_eq!(&iso[offset..offset + 5], b"CD001");
}

#[test]
fn build_iso_with_large_uki() {
    // ARRANGE
    let uki = fake_uki(16 * 1024 * 1024);

    // ACT
    let iso = miso::build_iso(&uki, Arch::X86_64).expect("build_iso must succeed for large UKI");

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
    let iso = miso::build_iso(&uki, Arch::X86_64).expect("build_iso must succeed");

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
    let iso = miso::build_iso(&uki, Arch::X86_64).expect("build_iso must succeed");

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
    let iso = miso::build_iso(&uki, Arch::X86_64).expect("build_iso must succeed");

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
    let iso = miso::build_iso(&uki, Arch::X86_64).expect("build_iso must succeed");

    // ASSERT
    let cat_start = SECTOR_SIZE * 21;
    assert_eq!(
        iso[cat_start + 32],
        0x88,
        "El Torito default entry boot indicator must be 0x88 (bootable)"
    );
}

#[test]
fn build_img_has_valid_gpt() {
    // ARRANGE
    let uki = fake_uki(1024);

    // ACT
    let img = miso::build_img(&uki, &[]).expect("build_img must succeed");

    // ASSERT
    let mut cursor = Cursor::new(img);
    let gpt = gptman::GPT::find_from(&mut cursor).expect("image must contain a valid GPT");
    assert!(
        gpt.iter().any(|(_, p)| p.is_used()),
        "GPT must have at least one partition"
    );
}

#[test]
fn build_img_has_protective_mbr() {
    // ARRANGE
    let uki = fake_uki(512);

    // ACT
    let img = miso::build_img(&uki, &[]).expect("build_img must succeed");

    // ASSERT
    assert_eq!(img[510], 0x55, "MBR byte 510 must be 0x55");
    assert_eq!(img[511], 0xAA, "MBR byte 511 must be 0xAA");
    assert_eq!(
        img[450], 0xEE,
        "MBR partition type must be 0xEE (GPT protective)"
    );
}

#[test]
fn build_img_esp_has_efi_system_partition_guid() {
    // ARRANGE
    let uki = fake_uki(1024);
    let efi_guid: [u8; 16] = [
        0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9,
        0x3b,
    ];

    // ACT
    let img = miso::build_img(&uki, &[]).expect("build_img must succeed");

    // ASSERT
    let mut cursor = Cursor::new(img);
    let gpt = gptman::GPT::find_from(&mut cursor).expect("valid GPT");
    let (_, part) = gpt
        .iter()
        .find(|(_, p)| p.is_used())
        .expect("must have partition");
    assert_eq!(part.partition_type_guid, efi_guid);
}

#[test]
fn build_img_aarch64_with_blobs_contains_fat_data() {
    // ARRANGE
    let uki = fake_uki(2048);
    let config = b"arm_64bit=1\n";
    let blobs: &[(&str, &[u8])] = &[("config.txt", config)];

    // ACT
    let img = miso::build_img(&uki, blobs).expect("build_img with blobs must succeed");

    // ASSERT
    let mut cursor = Cursor::new(&img);
    let gpt = gptman::GPT::find_from(&mut cursor).expect("valid GPT");
    let (_, part) = gpt
        .iter()
        .find(|(_, p)| p.is_used())
        .expect("must have partition");
    let offset = (part.starting_lba * 512) as usize;
    let esp_len = ((part.ending_lba - part.starting_lba + 1) * 512) as usize;
    let fat_data = &img[offset..offset + esp_len];
    let mut fat_cursor = Cursor::new(fat_data.to_vec());
    let fs = fatfs::FileSystem::new(&mut fat_cursor, fatfs::FsOptions::new())
        .expect("FAT32 must be valid");
    let root = fs.root_dir();

    let mut cfg_file = root.open_file("config.txt").expect("config.txt must exist");
    let mut cfg_content = Vec::new();
    std::io::Read::read_to_end(&mut cfg_file, &mut cfg_content).expect("read config.txt");
    assert_eq!(cfg_content, config);

    let mut uki_file = root
        .open_dir("EFI")
        .expect("EFI dir")
        .open_dir("BOOT")
        .expect("BOOT dir")
        .open_file("BOOTAA64.EFI")
        .expect("BOOTAA64.EFI must exist");
    let mut uki_content = Vec::new();
    std::io::Read::read_to_end(&mut uki_file, &mut uki_content).expect("read UKI");
    assert_eq!(uki_content, fake_uki(2048));
}

#[test]
fn build_img_disk_size_is_sector_aligned() {
    // ARRANGE
    let uki = fake_uki(4096);

    // ACT
    let img = miso::build_img(&uki, &[]).expect("build_img must succeed");

    // ASSERT
    assert_eq!(img.len() % 512, 0, "disk image size must be sector-aligned");
}

#[test]
fn build_img_partition_name_is_efi() {
    // ARRANGE
    let uki = fake_uki(512);

    // ACT
    let img = miso::build_img(&uki, &[]).expect("build_img must succeed");

    // ASSERT
    let mut cursor = Cursor::new(img);
    let gpt = gptman::GPT::find_from(&mut cursor).expect("valid GPT");
    let (_, part) = gpt
        .iter()
        .find(|(_, p)| p.is_used())
        .expect("must have partition");
    assert_eq!(part.partition_name.as_str(), "EFI");
}
