//! ISO 9660 + El Torito EFI bootable image writer.

use std::io::{Seek, SeekFrom, Write};

use crate::MisoError;

/// Logical block size for ISO 9660, mandated by ECMA-119.
pub const SECTOR_SIZE: usize = 2048;

/// Maximum El Torito EFI boot image size expressed in 2048-byte sectors.
const MAX_EL_TORITO_IMAGE_SECTORS: usize = u16::MAX as usize;

/// Offset of the El Torito boot catalog LBA field in the Boot Record VD (byte offset into sector).
const BOOT_RECORD_CATALOG_OFFSET: usize = 71;

/// LBA of the Primary Volume Descriptor (ECMA-119 §6.7.1).
const LBA_PVD: u64 = 16;
/// LBA of the Boot Record Volume Descriptor.
const LBA_BOOT_RECORD: u64 = LBA_PVD + 1;
/// LBA of the Volume Descriptor Set Terminator.
const LBA_VD_TERMINATOR: u64 = LBA_BOOT_RECORD + 1;
/// LBA of the L-path table.
const LBA_PATH_TABLE_L: u64 = LBA_VD_TERMINATOR + 1;
/// LBA of the M-path table.
const LBA_PATH_TABLE_M: u64 = LBA_PATH_TABLE_L + 1;
/// LBA of the El Torito boot catalog.
const LBA_BOOT_CATALOG: u64 = LBA_PATH_TABLE_M + 1;
/// LBA of the root directory record.
const LBA_ROOT_DIR: u64 = LBA_BOOT_CATALOG + 1;
/// LBA where file data begins.
const LBA_FILE_DATA: u64 = LBA_ROOT_DIR + 1;

/// Volume identifier written into the PVD (padded to 32 bytes by callers).
const SYSTEM_IDENTIFIER: &[u8; 32] = b"                                ";

/// Writes an ISO date/time field with all-zero (unspecified) values.
fn zero_date() -> [u8; 17] {
    let mut d = [b'0'; 17];
    d[16] = 0; // GMT offset byte
    d
}

/// Writes `value` in both little-endian and big-endian form (ISO 7.3.3).
fn both_endian_u32(buf: &mut [u8], offset: usize, value: u32) {
    buf[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    buf[offset + 4..offset + 8].copy_from_slice(&value.to_be_bytes());
}

/// Writes `value` in both little-endian and big-endian form (ISO 7.2.3).
fn both_endian_u16(buf: &mut [u8], offset: usize, value: u16) {
    buf[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    buf[offset + 2..offset + 4].copy_from_slice(&value.to_be_bytes());
}

/// Builds a single directory record for use inside the root directory sector.
fn directory_record(name_bytes: &[u8], lba: u32, size: u32, is_dir: bool) -> Vec<u8> {
    let name_len = name_bytes.len();
    let base_len = 33 + name_len;
    let record_len = if base_len.is_multiple_of(2) {
        base_len
    } else {
        base_len + 1
    };
    let mut rec = vec![0u8; record_len];
    rec[0] = record_len as u8;
    rec[1] = 0; // Extended attribute record length
    rec[2..6].copy_from_slice(&lba.to_le_bytes());
    rec[6..10].copy_from_slice(&lba.to_be_bytes());
    rec[10..14].copy_from_slice(&size.to_le_bytes());
    rec[14..18].copy_from_slice(&size.to_be_bytes());
    rec[18..25].copy_from_slice(&[0u8; 7]); // Recording date/time
    rec[25] = if is_dir { 0x02 } else { 0x00 }; // File flags
    rec[26] = 0; // File unit size
    rec[27] = 0; // Interleave gap size
    both_endian_u16(&mut rec, 28, 1); // Volume sequence number
    rec[32] = name_len as u8;
    rec[33..33 + name_len].copy_from_slice(name_bytes);
    rec
}

/// Builds the Primary Volume Descriptor sector (LBA 16, ECMA-119 §8.4).
fn build_pvd(total_sectors: u32, efi_image_size: u32) -> [u8; SECTOR_SIZE] {
    let mut pvd = [0u8; SECTOR_SIZE];
    pvd[0] = 1; // Type: Primary VD
    pvd[1..6].copy_from_slice(b"CD001");
    pvd[6] = 1; // Version
    pvd[8..40].copy_from_slice(SYSTEM_IDENTIFIER);
    pvd[40..72].copy_from_slice(&[b' '; 32]); // Volume identifier (unused)
    both_endian_u32(&mut pvd, 80, total_sectors);
    pvd[88] = 1; // Escape sequences
    both_endian_u16(&mut pvd, 120, 1); // Volume set size
    both_endian_u16(&mut pvd, 124, 1); // Volume sequence number
    both_endian_u16(&mut pvd, 128, SECTOR_SIZE as u16); // Logical block size
    // Path table size: one root entry = 8 + 1 (name "\x01") + 1 pad = 10 bytes
    both_endian_u32(&mut pvd, 132, 10);
    // L-path table LBA (little-endian only, §8.4.19)
    pvd[140..144].copy_from_slice(&(LBA_PATH_TABLE_L as u32).to_le_bytes());
    // M-path table LBA (big-endian only, §8.4.21)
    pvd[148..152].copy_from_slice(&(LBA_PATH_TABLE_M as u32).to_be_bytes());

    // Root directory record (34 bytes, §8.4.23)
    let root_size = root_dir_size(efi_image_size);
    let root = &mut pvd[156..190];
    root[0] = 34; // Length of directory record
    root[1] = 0;
    root[2..6].copy_from_slice(&(LBA_ROOT_DIR as u32).to_le_bytes());
    root[6..10].copy_from_slice(&(LBA_ROOT_DIR as u32).to_be_bytes());
    root[10..14].copy_from_slice(&(root_size as u32).to_le_bytes());
    root[14..18].copy_from_slice(&(root_size as u32).to_be_bytes());
    root[18..25].copy_from_slice(&[0u8; 7]);
    root[25] = 0x02; // Directory flag
    root[26] = 0;
    root[27] = 0;
    root[28..30].copy_from_slice(&1u16.to_le_bytes());
    root[30..32].copy_from_slice(&1u16.to_be_bytes());
    root[32] = 1; // File identifier length
    root[33] = 0x00; // Root directory identifier

    pvd[190..222].copy_from_slice(&[b' '; 32]); // Volume set identifier
    pvd[222..254].copy_from_slice(&[b' '; 32]); // Publisher identifier
    pvd[254..286].copy_from_slice(&[b' '; 32]); // Data preparer identifier
    pvd[286..318].copy_from_slice(&[b' '; 32]); // Application identifier
    pvd[318..446].copy_from_slice(&[b' '; 128]); // Copyright / abstract / biblio
    pvd[446..463].copy_from_slice(&zero_date()); // Volume creation
    pvd[463..480].copy_from_slice(&zero_date()); // Volume modification
    pvd[480..497].copy_from_slice(&zero_date()); // Volume expiration
    pvd[497..514].copy_from_slice(&zero_date()); // Volume effective
    pvd[514] = 1; // File structure version
    pvd
}

/// Builds the Boot Record Volume Descriptor sector (LBA 17, El Torito §2.1).
fn build_boot_record_vd() -> [u8; SECTOR_SIZE] {
    let mut vd = [0u8; SECTOR_SIZE];
    vd[0] = 0; // Type: Boot Record
    vd[1..6].copy_from_slice(b"CD001");
    vd[6] = 1; // Version
    vd[7..39].copy_from_slice(b"EL TORITO SPECIFICATION         "); // Boot system id (32 bytes)
    // Boot catalog LBA at offset 71 (El Torito §2.1)
    vd[BOOT_RECORD_CATALOG_OFFSET..BOOT_RECORD_CATALOG_OFFSET + 4]
        .copy_from_slice(&(LBA_BOOT_CATALOG as u32).to_le_bytes());
    vd
}

/// Builds the Volume Descriptor Set Terminator sector (LBA 18, ECMA-119 §8.3).
fn build_vd_terminator() -> [u8; SECTOR_SIZE] {
    let mut vd = [0u8; SECTOR_SIZE];
    vd[0] = 255; // Type: Terminator
    vd[1..6].copy_from_slice(b"CD001");
    vd[6] = 1;
    vd
}

/// Builds the L-path table sector (little-endian, ECMA-119 §9.4).
fn build_path_table_l() -> [u8; SECTOR_SIZE] {
    let mut pt = [0u8; SECTOR_SIZE];
    pt[0] = 1; // Length of directory identifier
    pt[1] = 0; // Extended attribute record length
    pt[2..6].copy_from_slice(&(LBA_ROOT_DIR as u32).to_le_bytes());
    pt[6..8].copy_from_slice(&1u16.to_le_bytes()); // Directory number of parent
    pt[8] = 0x00; // Root directory identifier
    pt[9] = 0x00; // Padding
    pt
}

/// Builds the M-path table sector (big-endian, ECMA-119 §9.4).
fn build_path_table_m() -> [u8; SECTOR_SIZE] {
    let mut pt = [0u8; SECTOR_SIZE];
    pt[0] = 1;
    pt[1] = 0;
    pt[2..6].copy_from_slice(&(LBA_ROOT_DIR as u32).to_be_bytes());
    pt[6..8].copy_from_slice(&1u16.to_be_bytes());
    pt[8] = 0x00;
    pt[9] = 0x00;
    pt
}

/// Builds the El Torito boot catalog sector (LBA 21, El Torito §2.2 & §2.3).
fn build_boot_catalog(efi_image_lba: u32, efi_image_sectors: u16) -> [u8; SECTOR_SIZE] {
    let mut cat = [0u8; SECTOR_SIZE];

    // Validation entry (§2.2): 0x01 header, platform 0xEF (EFI), checksum
    cat[0] = 0x01; // Header ID
    cat[1] = 0xEF; // Platform: EFI
    cat[2] = 0x00;
    cat[3] = 0x00;
    // Manufacturer ID: 24 bytes of spaces at offset 4
    for b in cat[4..28].iter_mut() {
        *b = b' ';
    }
    cat[30] = 0x55; // Key byte 1
    cat[31] = 0xAA; // Key byte 2

    // The two-byte checksum at offset 28 must make the sum of all 16 words (32 bytes) == 0 mod 0x10000
    let mut sum: u16 = 0;
    for chunk in cat[0..32].chunks(2) {
        sum = sum.wrapping_add(u16::from_le_bytes([chunk[0], chunk[1]]));
    }
    let checksum = (0u16).wrapping_sub(sum);
    cat[28..30].copy_from_slice(&checksum.to_le_bytes());

    // Initial/default entry (§2.3): EFI, no emulation
    cat[32] = 0x88; // Boot indicator: bootable
    cat[33] = 0x00; // Boot media type: no emulation
    cat[34] = 0x00; // Load segment: 0 (use default)
    cat[35] = 0x00;
    cat[36] = 0x00; // System type
    cat[37] = 0x00; // Unused
    cat[38..40].copy_from_slice(&efi_image_sectors.to_le_bytes()); // Sector count
    cat[40..44].copy_from_slice(&efi_image_lba.to_le_bytes()); // LBA of load image

    // Section header entry for EFI (§2.4)
    cat[64] = 0x91; // Header indicator: final section header, platform EFI
    cat[65] = 0xEF; // Platform: EFI
    cat[66..68].copy_from_slice(&1u16.to_le_bytes()); // Number of section entries
    // Section entry (§2.5) — same as default entry above
    cat[96] = 0x88;
    cat[97] = 0x00;
    cat[98] = 0x00;
    cat[99] = 0x00;
    cat[100] = 0x00;
    cat[101] = 0x00;
    cat[102..104].copy_from_slice(&efi_image_sectors.to_le_bytes());
    cat[104..108].copy_from_slice(&efi_image_lba.to_le_bytes());

    cat
}

/// Returns the total size of the root directory sector in bytes.
fn root_dir_size(efi_image_size: u32) -> usize {
    // Dot + dotdot + efiboot.img entry
    let dot = directory_record(&[0x00], LBA_ROOT_DIR as u32, SECTOR_SIZE as u32, true);
    let dotdot = directory_record(&[0x01], LBA_ROOT_DIR as u32, SECTOR_SIZE as u32, true);
    let efi = directory_record(
        b"EFIBOOT.IMG;1",
        LBA_FILE_DATA as u32,
        efi_image_size,
        false,
    );
    dot.len() + dotdot.len() + efi.len()
}

/// Builds the root directory sector containing entries for the FAT image.
fn build_root_dir(efi_image_size: u32) -> [u8; SECTOR_SIZE] {
    let mut dir = [0u8; SECTOR_SIZE];
    let dir_size = root_dir_size(efi_image_size) as u32;

    let dot = directory_record(&[0x00], LBA_ROOT_DIR as u32, dir_size, true);
    let dotdot = directory_record(&[0x01], LBA_ROOT_DIR as u32, dir_size, true);
    let efi = directory_record(
        b"EFIBOOT.IMG;1",
        LBA_FILE_DATA as u32,
        efi_image_size,
        false,
    );

    let mut offset = 0;
    dir[offset..offset + dot.len()].copy_from_slice(&dot);
    offset += dot.len();
    dir[offset..offset + dotdot.len()].copy_from_slice(&dotdot);
    offset += dotdot.len();
    dir[offset..offset + efi.len()].copy_from_slice(&efi);
    dir
}

/// Writes a protective MBR entry so the ISO doubles as a valid hybrid MBR disk image.
fn write_protective_mbr(
    out: &mut (impl Write + Seek),
    efi_image_offset_bytes: u64,
    efi_image_size_bytes: u64,
) -> Result<(), MisoError> {
    out.seek(SeekFrom::Start(446))?;
    let mut entry = [0u8; 16];
    entry[0] = 0x00; // Not bootable
    entry[4] = 0xEF; // Partition type: EFI System Partition
    // CHS values are set to 0xFEFFFF (max) when LBA addressing is used
    entry[1] = 0xFE;
    entry[2] = 0xFF;
    entry[3] = 0xFF;
    entry[5] = 0xFE;
    entry[6] = 0xFF;
    entry[7] = 0xFF;
    let start_lba = (efi_image_offset_bytes / 512) as u32;
    let size_lba = (efi_image_size_bytes / 512) as u32;
    entry[8..12].copy_from_slice(&start_lba.to_le_bytes());
    entry[12..16].copy_from_slice(&size_lba.to_le_bytes());
    out.write_all(&entry)?;

    out.seek(SeekFrom::Start(510))?;
    out.write_all(&[0x55, 0xAA])?;

    Ok(())
}

/// Returns the El Torito boot image sector count, rejecting oversized EFI images.
fn el_torito_sector_count(efi_image_len: usize) -> Result<u16, MisoError> {
    let efi_sectors = efi_image_len.div_ceil(SECTOR_SIZE);
    if efi_sectors > MAX_EL_TORITO_IMAGE_SECTORS {
        return Err(MisoError::Iso(format!(
            "EFI boot image too large for El Torito: {efi_sectors} sectors > {}",
            u16::MAX
        )));
    }

    Ok(efi_sectors as u16)
}

/// Writes a complete bootable ISO 9660 image.
pub fn write(out: &mut (impl Write + Seek), efi_image: &[u8]) -> Result<(), MisoError> {
    let efi_sectors = efi_image.len().div_ceil(SECTOR_SIZE);
    let efi_image_lba = LBA_FILE_DATA as u32;
    let efi_image_size = efi_image.len() as u32;
    let efi_image_sectors_u16 = el_torito_sector_count(efi_image.len())?;
    let total_sectors = (LBA_FILE_DATA + efi_sectors as u64 + 1) as u32;

    // System area: 16 empty sectors (bytes 0–32767)
    let system_area = vec![0u8; SECTOR_SIZE * LBA_PVD as usize];
    out.seek(SeekFrom::Start(0))?;
    out.write_all(&system_area)?;

    // Sector 16: Primary Volume Descriptor
    out.seek(SeekFrom::Start(LBA_PVD * SECTOR_SIZE as u64))?;
    out.write_all(&build_pvd(total_sectors, efi_image_size))?;

    // Sector 17: Boot Record Volume Descriptor
    out.seek(SeekFrom::Start(LBA_BOOT_RECORD * SECTOR_SIZE as u64))?;
    out.write_all(&build_boot_record_vd())?;

    // Sector 18: Volume Descriptor Set Terminator
    out.seek(SeekFrom::Start(LBA_VD_TERMINATOR * SECTOR_SIZE as u64))?;
    out.write_all(&build_vd_terminator())?;

    // Sector 19: L-path table
    out.seek(SeekFrom::Start(LBA_PATH_TABLE_L * SECTOR_SIZE as u64))?;
    out.write_all(&build_path_table_l())?;

    // Sector 20: M-path table
    out.seek(SeekFrom::Start(LBA_PATH_TABLE_M * SECTOR_SIZE as u64))?;
    out.write_all(&build_path_table_m())?;

    // Sector 21: El Torito boot catalog
    out.seek(SeekFrom::Start(LBA_BOOT_CATALOG * SECTOR_SIZE as u64))?;
    out.write_all(&build_boot_catalog(efi_image_lba, efi_image_sectors_u16))?;

    // Sector 22: Root directory
    out.seek(SeekFrom::Start(LBA_ROOT_DIR * SECTOR_SIZE as u64))?;
    out.write_all(&build_root_dir(efi_image_size))?;

    // Sector 23+: EFI image data (padded to sector boundary)
    out.seek(SeekFrom::Start(LBA_FILE_DATA * SECTOR_SIZE as u64))?;
    out.write_all(efi_image)?;
    let padding = efi_sectors * SECTOR_SIZE - efi_image.len();
    if padding > 0 {
        out.write_all(&vec![0u8; padding])?;
    }

    // GPT hybrid MBR entry at byte 446
    let efi_offset = LBA_FILE_DATA * SECTOR_SIZE as u64;
    write_protective_mbr(out, efi_offset, efi_image.len() as u64)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn minimal_efi_image() -> Vec<u8> {
        vec![0xEFu8; 4 * SECTOR_SIZE]
    }

    #[test]
    fn write_iso_places_cd001_magic_at_sector_16() {
        // ARRANGE
        let efi = minimal_efi_image();
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write(&mut buf, &efi).expect("write_iso must succeed");

        // ASSERT
        let data = buf.into_inner();
        let offset = LBA_PVD as usize * SECTOR_SIZE + 1;
        assert_eq!(
            &data[offset..offset + 5],
            b"CD001",
            "PVD must have CD001 magic"
        );
    }

    #[test]
    fn write_iso_pvd_type_byte_is_one() {
        // ARRANGE
        let efi = minimal_efi_image();
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write(&mut buf, &efi).expect("write_iso must succeed");

        // ASSERT
        let data = buf.into_inner();
        let pvd_start = LBA_PVD as usize * SECTOR_SIZE;
        assert_eq!(data[pvd_start], 1, "PVD type byte must be 1");
    }

    #[test]
    fn write_iso_boot_record_vd_type_byte_is_zero() {
        // ARRANGE
        let efi = minimal_efi_image();
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write(&mut buf, &efi).expect("write_iso must succeed");

        // ASSERT
        let data = buf.into_inner();
        let brvd_start = LBA_BOOT_RECORD as usize * SECTOR_SIZE;
        assert_eq!(data[brvd_start], 0, "Boot Record VD type byte must be 0");
    }

    #[test]
    fn write_iso_terminator_type_byte_is_255() {
        // ARRANGE
        let efi = minimal_efi_image();
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write(&mut buf, &efi).expect("write_iso must succeed");

        // ASSERT
        let data = buf.into_inner();
        let term_start = LBA_VD_TERMINATOR as usize * SECTOR_SIZE;
        assert_eq!(data[term_start], 255, "VD terminator type byte must be 255");
    }

    #[test]
    fn write_iso_boot_record_vd_has_el_torito_identifier() {
        // ARRANGE
        let efi = minimal_efi_image();
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write(&mut buf, &efi).expect("write_iso must succeed");

        // ASSERT
        let data = buf.into_inner();
        let brvd_start = LBA_BOOT_RECORD as usize * SECTOR_SIZE;
        assert_eq!(
            &data[brvd_start + 7..brvd_start + 39],
            b"EL TORITO SPECIFICATION         ",
            "Boot Record VD must contain El Torito identifier"
        );
    }

    #[test]
    fn write_iso_boot_catalog_lba_matches_in_boot_record_vd() {
        // ARRANGE
        let efi = minimal_efi_image();
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write(&mut buf, &efi).expect("write_iso must succeed");

        // ASSERT
        let data = buf.into_inner();
        let brvd_start = LBA_BOOT_RECORD as usize * SECTOR_SIZE;
        let catalog_lba = u32::from_le_bytes(
            data[brvd_start + BOOT_RECORD_CATALOG_OFFSET
                ..brvd_start + BOOT_RECORD_CATALOG_OFFSET + 4]
                .try_into()
                .expect("4-byte slice for catalog LBA"),
        );
        assert_eq!(catalog_lba, LBA_BOOT_CATALOG as u32);
    }

    #[test]
    fn write_iso_boot_catalog_validation_entry_checksum_is_valid() {
        // ARRANGE
        let efi = minimal_efi_image();
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write(&mut buf, &efi).expect("write_iso must succeed");

        // ASSERT
        let data = buf.into_inner();
        let cat_start = LBA_BOOT_CATALOG as usize * SECTOR_SIZE;
        let validation = &data[cat_start..cat_start + 32];
        let sum: u32 = validation
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]) as u32)
            .sum();
        assert_eq!(
            sum % 0x10000,
            0,
            "boot catalog validation checksum must be zero mod 0x10000"
        );
    }

    #[test]
    fn write_iso_boot_catalog_has_55aa_key_bytes() {
        // ARRANGE
        let efi = minimal_efi_image();
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write(&mut buf, &efi).expect("write_iso must succeed");

        // ASSERT
        let data = buf.into_inner();
        let cat_start = LBA_BOOT_CATALOG as usize * SECTOR_SIZE;
        assert_eq!(data[cat_start + 30], 0x55);
        assert_eq!(data[cat_start + 31], 0xAA);
    }

    #[test]
    fn write_iso_file_data_written_at_lba_file_data() {
        // ARRANGE
        let efi = vec![0xBEu8; SECTOR_SIZE];
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write(&mut buf, &efi).expect("write_iso must succeed");

        // ASSERT
        let data = buf.into_inner();
        let offset = LBA_FILE_DATA as usize * SECTOR_SIZE;
        assert_eq!(&data[offset..offset + SECTOR_SIZE], efi.as_slice());
    }

    #[test]
    fn write_iso_output_size_is_sector_aligned() {
        // ARRANGE
        let efi = vec![0u8; 3 * SECTOR_SIZE + 100];
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write(&mut buf, &efi).expect("write_iso must succeed");

        // ASSERT
        let len = buf.into_inner().len();
        assert_eq!(len % SECTOR_SIZE, 0, "ISO size must be sector-aligned");
    }

    #[test]
    fn write_iso_mbr_has_boot_signature() {
        // ARRANGE
        let efi = minimal_efi_image();
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write(&mut buf, &efi).expect("write_iso must succeed");

        // ASSERT
        let data = buf.into_inner();
        assert_eq!(data[510], 0x55, "MBR byte 510 must be 0x55");
        assert_eq!(data[511], 0xAA, "MBR byte 511 must be 0xAA");
    }

    #[test]
    fn write_iso_mbr_partition_entry_type_is_ef() {
        // ARRANGE
        let efi = minimal_efi_image();
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write(&mut buf, &efi).expect("write_iso must succeed");

        // ASSERT
        let data = buf.into_inner();
        // MBR partition type byte is at offset 446 + 4
        assert_eq!(data[450], 0xEF, "MBR partition type must be 0xEF (EFI)");
    }

    #[test]
    fn directory_record_file_has_correct_length_and_name() {
        // ARRANGE
        let name = b"EFIBOOT.IMG;1";

        // ACT
        let rec = directory_record(name, 23, 8192, false);

        // ASSERT
        let expected_base = 33 + name.len();
        let expected_len = if expected_base.is_multiple_of(2) {
            expected_base
        } else {
            expected_base + 1
        };
        assert_eq!(
            rec[0] as usize, expected_len,
            "record length field must match"
        );
        assert_eq!(&rec[33..33 + name.len()], name);
        assert_eq!(rec[25], 0x00, "file flag must be 0 for a file");
    }

    #[test]
    fn directory_record_dir_flag_is_set() {
        // ARRANGE / ACT
        let rec = directory_record(&[0x00], 22, 2048, true);

        // ASSERT
        assert_eq!(rec[25], 0x02, "directory flag must be 0x02");
    }

    #[test]
    fn zero_date_has_correct_length() {
        // ARRANGE / ACT
        let d = zero_date();

        // ASSERT
        assert_eq!(d.len(), 17);
    }

    #[test]
    fn el_torito_sector_count_rejects_oversized_image() {
        // ARRANGE
        let oversized = (MAX_EL_TORITO_IMAGE_SECTORS + 1) * SECTOR_SIZE;

        // ACT
        let err = el_torito_sector_count(oversized).expect_err("oversized image must fail");

        // ASSERT
        assert!(matches!(err, MisoError::Iso(_)));
        assert!(err.to_string().contains("too large for El Torito"));
    }
}
