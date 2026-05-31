//! ISO 9660 + El Torito EFI bootable image writer.

use std::io::{Seek, SeekFrom, Write};

use parttable::mbr;
use parttable::mbr::types::{MBR_EFI_SYSTEM_TYPE, MbrPartitionEntry};

use crate::error::{MisoError, Result};

/// Logical block size for ISO 9660, mandated by ECMA-119.
pub const SECTOR_SIZE: usize = 2048;

const LBA_PVD_USIZE: usize = 16;

/// Offset of the El Torito boot catalog LBA field in the Boot Record VD (byte offset into sector).
const BOOT_RECORD_CATALOG_OFFSET: usize = 71;

/// LBA of the Primary Volume Descriptor (ECMA-119 §6.7.1).
const LBA_PVD: u32 = 16;
/// LBA of the Boot Record Volume Descriptor.
const LBA_BOOT_RECORD: u32 = LBA_PVD + 1;
/// LBA of the Volume Descriptor Set Terminator.
const LBA_VD_TERMINATOR: u32 = LBA_BOOT_RECORD + 1;
/// LBA of the L-path table.
const LBA_PATH_TABLE_L: u32 = LBA_VD_TERMINATOR + 1;
/// LBA of the M-path table.
const LBA_PATH_TABLE_M: u32 = LBA_PATH_TABLE_L + 1;
/// LBA of the El Torito boot catalog.
const LBA_BOOT_CATALOG: u32 = LBA_PATH_TABLE_M + 1;
/// LBA of the root directory record.
const LBA_ROOT_DIR: u32 = LBA_BOOT_CATALOG + 1;
/// LBA where file data begins.
const LBA_FILE_DATA: u32 = LBA_ROOT_DIR + 1;

/// Volume identifier written into the PVD (padded to 32 bytes by callers).
const SYSTEM_IDENTIFIER: &[u8; 32] = b"                                ";

/// Writes a complete bootable ISO 9660 image.
///
/// # Errors
///
/// Returns an error if ISO metadata construction overflows format limits or if any
/// write or seek operation fails.
pub fn write<W: Write + Seek>(out: &mut W, efi_image: &[u8]) -> Result<()> {
    let efi_sectors = efi_image.len().div_ceil(SECTOR_SIZE);
    let efi_image_lba = LBA_FILE_DATA;
    let efi_image_size = match u32::try_from(efi_image.len()) {
        Ok(efi_image_size) => efi_image_size,
        Err(_conversion_error) => {
            return Err(MisoError::Iso("ISO field must fit in u32".to_owned()));
        }
    };
    let efi_image_sectors_u16 = el_torito_sector_count(efi_image.len())?;
    let efi_sectors_u32 = u32::try_from(efi_sectors).unwrap_or(u32::MAX);
    let total_sectors = LBA_FILE_DATA
        .checked_add(efi_sectors_u32)
        .ok_or(MisoError::Iso(
            "u32 addition for ISO sectors overflowed".to_owned(),
        ))?
        .checked_add(1)
        .ok_or(MisoError::Iso(
            "u32 addition for ISO sectors overflowed".to_owned(),
        ))?;

    // System area: 16 empty sectors (bytes 0–32767)
    let system_area = vec![
        0_u8;
        SECTOR_SIZE
            .checked_mul(LBA_PVD_USIZE)
            .ok_or(MisoError::Iso(
                "usize multiplication for ISO structures overflowed".to_owned(),
            ))?
    ];
    out.seek(SeekFrom::Start(0))?;
    out.write_all(&system_area)?;

    // Sector 16: Primary Volume Descriptor
    out.seek(SeekFrom::Start(sector_offset(LBA_PVD)?))?;
    out.write_all(&build_pvd(total_sectors, efi_image_size)?)?;

    // Sector 17: Boot Record Volume Descriptor
    out.seek(SeekFrom::Start(sector_offset(LBA_BOOT_RECORD)?))?;
    out.write_all(&build_boot_record_vd())?;

    // Sector 18: Volume Descriptor Set Terminator
    out.seek(SeekFrom::Start(sector_offset(LBA_VD_TERMINATOR)?))?;
    out.write_all(&build_vd_terminator())?;

    // Sector 19: L-path table
    out.seek(SeekFrom::Start(sector_offset(LBA_PATH_TABLE_L)?))?;
    out.write_all(&build_path_table_l())?;

    // Sector 20: M-path table
    out.seek(SeekFrom::Start(sector_offset(LBA_PATH_TABLE_M)?))?;
    out.write_all(&build_path_table_m())?;

    // Sector 21: El Torito boot catalog
    out.seek(SeekFrom::Start(sector_offset(LBA_BOOT_CATALOG)?))?;
    out.write_all(&build_boot_catalog(efi_image_lba, efi_image_sectors_u16)?)?;

    // Sector 22: Root directory
    out.seek(SeekFrom::Start(sector_offset(LBA_ROOT_DIR)?))?;
    out.write_all(&build_root_dir(efi_image_size)?)?;

    // Sector 23+: EFI image data (padded to sector boundary)
    out.seek(SeekFrom::Start(sector_offset(LBA_FILE_DATA)?))?;
    out.write_all(efi_image)?;
    let padding = efi_sectors
        .checked_mul(SECTOR_SIZE)
        .ok_or(MisoError::Iso(
            "usize multiplication for ISO structures overflowed".to_owned(),
        ))?
        .checked_sub(efi_image.len())
        .ok_or(MisoError::Iso(
            "padded ISO image length underflowed".to_owned(),
        ))?;
    if padding > 0 {
        let padding_bytes = vec![0_u8; padding];
        out.write_all(&padding_bytes)?;
    }

    // GPT hybrid MBR entry at byte 446
    let efi_offset = sector_offset(LBA_FILE_DATA)?;
    write_protective_mbr(out, efi_offset, u64::from(efi_image_size))?;

    Ok(())
}

/// Returns the El Torito boot image sector count, rejecting oversized EFI images.
fn el_torito_sector_count(efi_image_len: usize) -> Result<u16> {
    let efi_sectors = efi_image_len.div_ceil(SECTOR_SIZE);
    if efi_sectors > usize::from(u16::MAX) {
        return Err(MisoError::Iso(format!(
            "EFI boot image too large for El Torito: {efi_sectors} sectors > {}",
            u16::MAX
        )));
    }

    Ok(u16::try_from(efi_sectors).unwrap_or(u16::MAX))
}

/// Builds the Primary Volume Descriptor sector (LBA 16, ECMA-119 §8.4).
fn build_pvd(total_sectors: u32, efi_image_size: u32) -> Result<[u8; SECTOR_SIZE]> {
    let mut pvd = [0_u8; SECTOR_SIZE];

    write_byte(&mut pvd, 0, 1);
    write_bytes(&mut pvd, 1, b"CD001");
    write_byte(&mut pvd, 6, 1);
    write_bytes(&mut pvd, 8, SYSTEM_IDENTIFIER);
    write_bytes(&mut pvd, 40, &[b' '; 32]);
    both_endian_u32(&mut pvd, 80, total_sectors);
    write_byte(&mut pvd, 88, 1);
    both_endian_u16(&mut pvd, 120, 1);
    both_endian_u16(&mut pvd, 124, 1);
    both_endian_u16(
        &mut pvd,
        128,
        u16::try_from(SECTOR_SIZE).unwrap_or(u16::MAX),
    );
    // Path table size: one root entry = 8 + 1 (name "\x01") + 1 pad = 10 bytes
    both_endian_u32(&mut pvd, 132, 10);
    // L-path table LBA (little-endian only, §8.4.19)
    write_bytes(&mut pvd, 140, &LBA_PATH_TABLE_L.to_le_bytes());
    // M-path table LBA (big-endian only, §8.4.21)
    write_bytes(&mut pvd, 148, &LBA_PATH_TABLE_M.to_be_bytes());

    // Root directory record (34 bytes, §8.4.23)
    let root_dir_bytes = root_dir_size(efi_image_size)?;
    let root = directory_record(&[0x00], LBA_ROOT_DIR, root_dir_bytes, true)?;
    write_bytes(&mut pvd, 156, &root);

    write_bytes(&mut pvd, 190, &[b' '; 32]);
    write_bytes(&mut pvd, 222, &[b' '; 32]);
    write_bytes(&mut pvd, 254, &[b' '; 32]);
    write_bytes(&mut pvd, 286, &[b' '; 32]);
    write_bytes(&mut pvd, 318, &[b' '; 128]);
    write_bytes(&mut pvd, 446, &zero_date());
    write_bytes(&mut pvd, 463, &zero_date());
    write_bytes(&mut pvd, 480, &zero_date());
    write_bytes(&mut pvd, 497, &zero_date());
    write_byte(&mut pvd, 514, 1);
    Ok(pvd)
}

/// Builds the Boot Record Volume Descriptor sector (LBA 17, El Torito §2.1).
fn build_boot_record_vd() -> [u8; SECTOR_SIZE] {
    let mut vd = [0_u8; SECTOR_SIZE];
    write_byte(&mut vd, 0, 0);
    write_bytes(&mut vd, 1, b"CD001");
    write_byte(&mut vd, 6, 1);
    write_bytes(&mut vd, 7, b"EL TORITO SPECIFICATION         ");
    // Boot catalog LBA at offset 71 (El Torito §2.1)
    write_bytes(
        &mut vd,
        BOOT_RECORD_CATALOG_OFFSET,
        &LBA_BOOT_CATALOG.to_le_bytes(),
    );
    vd
}

/// Builds the Volume Descriptor Set Terminator sector (LBA 18, ECMA-119 §8.3).
fn build_vd_terminator() -> [u8; SECTOR_SIZE] {
    let mut vd = [0_u8; SECTOR_SIZE];
    write_byte(&mut vd, 0, 255);
    write_bytes(&mut vd, 1, b"CD001");
    write_byte(&mut vd, 6, 1);
    vd
}

/// Builds the L-path table sector (little-endian, ECMA-119 §9.4).
fn build_path_table_l() -> [u8; SECTOR_SIZE] {
    let mut pt = [0_u8; SECTOR_SIZE];
    write_byte(&mut pt, 0, 1);
    write_byte(&mut pt, 1, 0);
    write_bytes(&mut pt, 2, &LBA_ROOT_DIR.to_le_bytes());
    write_bytes(&mut pt, 6, &1_u16.to_le_bytes());
    write_byte(&mut pt, 8, 0x00);
    write_byte(&mut pt, 9, 0x00);
    pt
}

/// Builds the M-path table sector (big-endian, ECMA-119 §9.4).
fn build_path_table_m() -> [u8; SECTOR_SIZE] {
    let mut pt = [0_u8; SECTOR_SIZE];
    write_byte(&mut pt, 0, 1);
    write_byte(&mut pt, 1, 0);
    write_bytes(&mut pt, 2, &LBA_ROOT_DIR.to_be_bytes());
    write_bytes(&mut pt, 6, &1_u16.to_be_bytes());
    write_byte(&mut pt, 8, 0x00);
    write_byte(&mut pt, 9, 0x00);
    pt
}

/// Builds the El Torito boot catalog sector (LBA 21, El Torito §2.2 & §2.3).
fn build_boot_catalog(efi_image_lba: u32, efi_image_sectors: u16) -> Result<[u8; SECTOR_SIZE]> {
    let mut cat = [0_u8; SECTOR_SIZE];

    // Validation entry (§2.2): 0x01 header, platform 0xEF (EFI), checksum
    write_byte(&mut cat, 0, 0x01);
    write_byte(&mut cat, 1, 0xEF);
    write_byte(&mut cat, 2, 0x00);
    write_byte(&mut cat, 3, 0x00);
    // Manufacturer ID: 24 bytes of spaces at offset 4
    for manufacturer_byte in &mut cat[4..28] {
        *manufacturer_byte = b' ';
    }
    write_byte(&mut cat, 30, 0x55);
    write_byte(&mut cat, 31, 0xAA);

    // The two-byte checksum at offset 28 must make the sum of all 16 words (32 bytes) == 0 mod 0x10000
    let mut sum: u16 = 0;
    for chunk in cat[0..32].chunks(2) {
        sum = sum.wrapping_add(checksum_word(chunk)?);
    }
    let checksum = (0_u16).wrapping_sub(sum);
    write_bytes(&mut cat, 28, &checksum.to_le_bytes());

    // Initial/default entry (§2.3): EFI, no emulation
    write_byte(&mut cat, 32, 0x88);
    write_byte(&mut cat, 33, 0x00);
    write_byte(&mut cat, 34, 0x00);
    write_byte(&mut cat, 35, 0x00);
    write_byte(&mut cat, 36, 0x00);
    write_byte(&mut cat, 37, 0x00);
    write_bytes(&mut cat, 38, &efi_image_sectors.to_le_bytes());
    write_bytes(&mut cat, 40, &efi_image_lba.to_le_bytes());

    // Section header entry for EFI (§2.4)
    write_byte(&mut cat, 64, 0x91);
    write_byte(&mut cat, 65, 0xEF);
    write_bytes(&mut cat, 66, &1_u16.to_le_bytes());
    // Section entry (§2.5) — same as default entry above
    write_byte(&mut cat, 96, 0x88);
    write_byte(&mut cat, 97, 0x00);
    write_byte(&mut cat, 98, 0x00);
    write_byte(&mut cat, 99, 0x00);
    write_byte(&mut cat, 100, 0x00);
    write_byte(&mut cat, 101, 0x00);
    write_bytes(&mut cat, 102, &efi_image_sectors.to_le_bytes());
    write_bytes(&mut cat, 104, &efi_image_lba.to_le_bytes());

    Ok(cat)
}

/// Builds a single directory record for use inside the root directory sector.
fn directory_record(name_bytes: &[u8], lba: u32, size: u32, is_dir: bool) -> Result<Vec<u8>> {
    let name_len = name_bytes.len();
    if name_len > usize::from(u8::MAX) {
        return Err(MisoError::Iso("ISO field must fit in u8".to_owned()));
    }

    let base_len = 33_usize.checked_add(name_len).ok_or(MisoError::Iso(
        "usize addition for ISO structures overflowed".to_owned(),
    ))?;
    let record_len = if base_len.is_multiple_of(2) {
        base_len
    } else {
        base_len.checked_add(1).ok_or(MisoError::Iso(
            "usize addition for ISO structures overflowed".to_owned(),
        ))?
    };
    let mut rec = vec![0_u8; record_len];
    write_byte(
        &mut rec,
        0,
        match u8::try_from(record_len) {
            Ok(record_len) => record_len,
            Err(_conversion_error) => {
                return Err(MisoError::Iso("ISO field must fit in u8".to_owned()));
            }
        },
    );
    write_byte(&mut rec, 1, 0);
    write_bytes(&mut rec, 2, &lba.to_le_bytes());
    write_bytes(&mut rec, 6, &lba.to_be_bytes());
    write_bytes(&mut rec, 10, &size.to_le_bytes());
    write_bytes(&mut rec, 14, &size.to_be_bytes());
    write_bytes(&mut rec, 18, &[0_u8; 7]);
    write_byte(&mut rec, 25, if is_dir { 0x02 } else { 0x00 });
    write_byte(&mut rec, 26, 0);
    write_byte(&mut rec, 27, 0);
    both_endian_u16(&mut rec, 28, 1);
    write_byte(&mut rec, 32, u8::try_from(name_len).unwrap_or(u8::MAX));
    write_bytes(&mut rec, 33, name_bytes);
    Ok(rec)
}

/// Builds the root directory sector containing entries for the FAT image.
fn build_root_dir(efi_image_size: u32) -> Result<[u8; SECTOR_SIZE]> {
    let mut dir = [0_u8; SECTOR_SIZE];
    let dir_size = root_dir_size(efi_image_size)?;

    let dot = directory_record(&[0x00], LBA_ROOT_DIR, dir_size, true)?;
    let dotdot = directory_record(&[0x01], LBA_ROOT_DIR, dir_size, true)?;
    let efi = directory_record(b"EFIBOOT.IMG;1", LBA_FILE_DATA, efi_image_size, false)?;

    let mut offset = 0;
    write_bytes(&mut dir, offset, &dot);
    offset = offset.checked_add(dot.len()).ok_or(MisoError::Iso(
        "usize addition for ISO structures overflowed".to_owned(),
    ))?;
    write_bytes(&mut dir, offset, &dotdot);
    offset = offset.checked_add(dotdot.len()).ok_or(MisoError::Iso(
        "usize addition for ISO structures overflowed".to_owned(),
    ))?;
    write_bytes(&mut dir, offset, &efi);
    Ok(dir)
}

/// Returns the total size of the root directory sector in bytes.
fn root_dir_size(efi_image_size: u32) -> Result<u32> {
    // Dot + dotdot records are always 34 bytes in ISO 9660.
    let efi = directory_record(b"EFIBOOT.IMG;1", LBA_FILE_DATA, efi_image_size, false)?;
    let efi_len = efi.first().copied().ok_or(MisoError::Iso(
        "directory record must contain a length byte".to_owned(),
    ))?;

    let size = 34_u32
        .checked_add(34)
        .and_then(|size| size.checked_add(u32::from(efi_len)))
        .ok_or(MisoError::Iso(
            "usize addition for ISO structures overflowed".to_owned(),
        ))?;

    Ok(size)
}

/// Writes a protective MBR entry so the ISO doubles as a valid hybrid MBR disk image.
fn write_protective_mbr<W: Write + Seek>(
    out: &mut W,
    efi_image_offset_bytes: u64,
    efi_image_size_bytes: u64,
) -> Result<()> {
    let start_lba = match u32::try_from(efi_image_offset_bytes >> 9) {
        Ok(start_lba) => start_lba,
        Err(_conversion_error) => {
            return Err(MisoError::Iso(
                "EFI image offset sectors must fit in u32".to_owned(),
            ));
        }
    };
    let size_lba = match u32::try_from(efi_image_size_bytes >> 9) {
        Ok(size_lba) => size_lba,
        Err(_conversion_error) => {
            return Err(MisoError::Iso(
                "EFI image size sectors must fit in u32".to_owned(),
            ));
        }
    };
    let entry = MbrPartitionEntry {
        bootable: false,
        partition_type: MBR_EFI_SYSTEM_TYPE,
        starting_lba: start_lba,
        size_lba,
    };

    mbr::io::write_entry(out, 0, &entry)?;
    mbr::io::write_signature(out)?;

    Ok(())
}

/// Writes an ISO date/time field with all-zero (unspecified) values.
fn zero_date() -> [u8; 17] {
    let mut date = [b'0'; 17];
    date[16] = 0;
    date
}

fn write_bytes(buf: &mut [u8], offset: usize, bytes: &[u8]) {
    let (_, tail) = buf.split_at_mut(offset);
    let (dst, _) = tail.split_at_mut(bytes.len());
    dst.copy_from_slice(bytes);
}

fn write_byte(buf: &mut [u8], offset: usize, value: u8) {
    write_bytes(buf, offset, &[value]);
}

fn sector_offset(lba: u32) -> Result<u64> {
    u64::from(lba)
        .checked_mul(u64::try_from(SECTOR_SIZE).unwrap_or(u64::MAX))
        .ok_or(MisoError::Iso(
            "u64 multiplication for ISO offsets overflowed".to_owned(),
        ))
}

fn checksum_word(chunk: &[u8]) -> Result<u16> {
    let pair: [u8; 2] = match chunk.try_into() {
        Ok(pair) => pair,
        Err(_chunk_error) => {
            return Err(MisoError::Iso(
                "boot catalog checksum chunk must be 2 bytes".to_owned(),
            ));
        }
    };
    Ok(u16::from_le_bytes(pair))
}

/// Writes `value` in both little-endian and big-endian form (ISO 7.3.3).
fn both_endian_u32(buf: &mut [u8], offset: usize, value: u32) {
    let (_, tail) = buf.split_at_mut(offset);
    let (field, _) = tail.split_at_mut(8);
    let (little_endian, big_endian) = field.split_at_mut(4);
    little_endian.copy_from_slice(&value.to_le_bytes());
    big_endian.copy_from_slice(&value.to_be_bytes());
}

/// Writes `value` in both little-endian and big-endian form (ISO 7.2.3).
fn both_endian_u16(buf: &mut [u8], offset: usize, value: u16) {
    let (_, tail) = buf.split_at_mut(offset);
    let (field, _) = tail.split_at_mut(4);
    let (little_endian, big_endian) = field.split_at_mut(2);
    little_endian.copy_from_slice(&value.to_le_bytes());
    big_endian.copy_from_slice(&value.to_be_bytes());
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::error::MisoError;

    fn minimal_efi_image() -> Vec<u8> {
        vec![0xEF_u8; 4 * SECTOR_SIZE]
    }

    fn sector_start(lba: u32) -> usize {
        usize::try_from(lba)
            .expect("LBA must fit in usize")
            .checked_mul(SECTOR_SIZE)
            .expect("sector offset must fit in usize")
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
        let offset = sector_start(LBA_PVD) + 1;
        assert_eq!(
            data.get(offset..offset + 5).expect("PVD magic must exist"),
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
        let pvd_start = sector_start(LBA_PVD);
        assert_eq!(
            data.get(pvd_start).copied().expect("PVD type must exist"),
            1,
            "PVD type byte must be 1"
        );
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
        let brvd_start = sector_start(LBA_BOOT_RECORD);
        assert_eq!(
            data.get(brvd_start)
                .copied()
                .expect("Boot Record VD type must exist"),
            0,
            "Boot Record VD type byte must be 0"
        );
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
        let term_start = sector_start(LBA_VD_TERMINATOR);
        assert_eq!(
            data.get(term_start)
                .copied()
                .expect("VD terminator type must exist"),
            255,
            "VD terminator type byte must be 255"
        );
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
        let brvd_start = sector_start(LBA_BOOT_RECORD);
        assert_eq!(
            data.get(brvd_start + 7..brvd_start + 39)
                .expect("El Torito identifier must exist"),
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
        let brvd_start = sector_start(LBA_BOOT_RECORD);
        let catalog_lba = u32::from_le_bytes(
            data.get(
                brvd_start + BOOT_RECORD_CATALOG_OFFSET
                    ..brvd_start + BOOT_RECORD_CATALOG_OFFSET + 4,
            )
            .expect("catalog LBA bytes must exist")
            .try_into()
            .expect("4-byte slice for catalog LBA"),
        );
        assert_eq!(catalog_lba, LBA_BOOT_CATALOG);
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
        let cat_start = sector_start(LBA_BOOT_CATALOG);
        let validation = data
            .get(cat_start..cat_start + 32)
            .expect("validation entry must exist");
        let sum: u32 = validation
            .chunks_exact(2)
            .map(|chunk| u32::from(checksum_word(chunk).expect("checksum word must build")))
            .sum();
        assert_eq!(
            sum.rem_euclid(0x10000),
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
        let cat_start = sector_start(LBA_BOOT_CATALOG);
        assert_eq!(
            data.get(cat_start + 30)
                .copied()
                .expect("first catalog key byte must exist"),
            0x55
        );
        assert_eq!(
            data.get(cat_start + 31)
                .copied()
                .expect("second catalog key byte must exist"),
            0xAA
        );
    }

    #[test]
    fn write_iso_file_data_written_at_lba_file_data() {
        // ARRANGE
        let efi = vec![0xBE_u8; SECTOR_SIZE];
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write(&mut buf, &efi).expect("write_iso must succeed");

        // ASSERT
        let data = buf.into_inner();
        let offset = sector_start(LBA_FILE_DATA);
        assert_eq!(
            data.get(offset..offset + SECTOR_SIZE)
                .expect("file data sector must exist"),
            efi.as_slice()
        );
    }

    #[test]
    fn write_iso_output_size_is_sector_aligned() {
        // ARRANGE
        let efi = vec![0_u8; 3 * SECTOR_SIZE + 100];
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write(&mut buf, &efi).expect("write_iso must succeed");

        // ASSERT
        let len = buf.into_inner().len();
        assert_eq!(
            len.rem_euclid(SECTOR_SIZE),
            0,
            "ISO size must be sector-aligned"
        );
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
        assert_eq!(
            data.get(510).copied().expect("MBR byte 510 must exist"),
            0x55,
            "MBR byte 510 must be 0x55"
        );
        assert_eq!(
            data.get(511).copied().expect("MBR byte 511 must exist"),
            0xAA,
            "MBR byte 511 must be 0xAA"
        );
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
        assert_eq!(
            data.get(450)
                .copied()
                .expect("MBR partition type byte must exist"),
            0xEF,
            "MBR partition type must be 0xEF (EFI)"
        );
    }

    #[test]
    fn directory_record_pads_odd_length_file_records_to_even_length() {
        // ARRANGE
        let name = b"AB;1";

        // ACT
        let rec = directory_record(name, 23, 8192, false).expect("directory record must build");

        // ASSERT
        let expected_base = 33 + name.len();
        let expected_len = expected_base + 1;
        assert_eq!(
            usize::from(
                rec.first()
                    .copied()
                    .expect("directory record length byte must exist"),
            ),
            expected_len,
            "record length field must match"
        );
        assert_eq!(
            rec.get(33..33 + name.len())
                .expect("directory record name must exist"),
            name
        );
        assert_eq!(
            rec.get(25)
                .copied()
                .expect("directory record file flag must exist"),
            0x00,
            "file flag must be 0 for a file"
        );
        assert_eq!(
            rec.len().rem_euclid(2),
            0,
            "directory record length must be even"
        );
        assert_eq!(
            rec.get(expected_len - 1)
                .copied()
                .expect("directory record padding byte must exist"),
            0,
            "padding byte must be zero-filled"
        );
    }

    #[test]
    fn directory_record_dir_flag_is_set() {
        // ARRANGE / ACT
        let rec = directory_record(&[0x00], 22, 2048, true).expect("directory record must build");

        // ASSERT
        assert_eq!(
            rec.get(25)
                .copied()
                .expect("directory record flag must exist"),
            0x02,
            "directory flag must be 0x02"
        );
    }

    #[test]
    fn zero_date_has_correct_length() {
        // ARRANGE / ACT
        let date = zero_date();

        // ASSERT
        assert_eq!(date.len(), 17);
    }

    #[test]
    fn el_torito_sector_count_rejects_oversized_image() {
        // ARRANGE
        let oversized = (usize::from(u16::MAX) + 1) * SECTOR_SIZE;

        // ACT
        let err = el_torito_sector_count(oversized).expect_err("oversized image must fail");

        // ASSERT
        assert!(matches!(err, MisoError::Iso(_)));
        assert!(err.to_string().contains("too large for El Torito"));
    }

    #[test]
    fn directory_record_rejects_name_lengths_that_do_not_fit_in_u8() {
        // ARRANGE
        let too_long_name = vec![b'A'; usize::from(u8::MAX) + 1];

        // ACT
        let err = directory_record(&too_long_name, 23, 8192, false)
            .expect_err("oversized file name must fail");

        // ASSERT
        assert!(matches!(err, MisoError::Iso(_)));
        assert!(err.to_string().contains("must fit in u8"));
    }

    #[test]
    fn directory_record_rejects_record_lengths_that_do_not_fit_in_u8() {
        // ARRANGE
        let too_large_record_name = vec![b'A'; 223];

        // ACT
        let err = directory_record(&too_large_record_name, 23, 8192, false)
            .expect_err("oversized record length must fail");

        // ASSERT
        assert!(matches!(err, MisoError::Iso(_)));
        assert!(err.to_string().contains("must fit in u8"));
    }

    #[test]
    fn root_dir_size_counts_dot_dotdot_and_efi_records() {
        // ARRANGE
        let efi_image_size = 4 * 1024;

        // ACT
        let size = root_dir_size(efi_image_size).expect("root directory size must build");

        // ASSERT
        assert_eq!(
            size,
            34 + 34 + 46,
            "root directory size must sum all record lengths"
        );
    }

    #[test]
    fn write_iso_rejects_sizes_that_do_not_fit_in_u32() {
        // ARRANGE
        let oversized = vec![0_u8; usize::try_from(u32::MAX).unwrap() + 1];
        let mut out = Cursor::new(Vec::new());

        // ACT
        let err = write(&mut out, &oversized).expect_err("oversized image must fail");

        // ASSERT
        assert!(matches!(err, MisoError::Iso(_)));
        assert!(err.to_string().contains("must fit in u32"));
    }

    #[test]
    fn checksum_word_rejects_non_word_sized_chunks() {
        // ARRANGE
        let short_chunk = [0xAA_u8];

        // ACT
        let err = checksum_word(&short_chunk).expect_err("short chunk must fail");

        // ASSERT
        assert!(matches!(err, MisoError::Iso(_)));
        assert!(err.to_string().contains("2 bytes"));
    }

    #[test]
    fn write_protective_mbr_rejects_offsets_that_do_not_fit_in_u32_sectors() {
        // ARRANGE
        let mut out = Cursor::new(Vec::new());
        let overflowing_offset = (u64::from(u32::MAX) + 1) << 9;

        // ACT
        let err = write_protective_mbr(&mut out, overflowing_offset, 512)
            .expect_err("oversized offset must fail");

        // ASSERT
        assert!(matches!(err, MisoError::Iso(_)));
        assert!(err.to_string().contains("offset sectors must fit in u32"));
    }

    #[test]
    fn write_protective_mbr_rejects_sizes_that_do_not_fit_in_u32_sectors() {
        // ARRANGE
        let mut out = Cursor::new(Vec::new());
        let overflowing_size = (u64::from(u32::MAX) + 1) << 9;

        // ACT
        let err = write_protective_mbr(&mut out, 512, overflowing_size)
            .expect_err("oversized size must fail");

        // ASSERT
        assert!(matches!(err, MisoError::Iso(_)));
        assert!(err.to_string().contains("size sectors must fit in u32"));
    }
}
