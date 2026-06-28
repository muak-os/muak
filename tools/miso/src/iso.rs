//! ISO 9660 + El Torito EFI bootable image writer.

use std::io::Write;

use parttable::mbr::io::mbr_bytes;
use parttable::mbr::types::{MBR_EFI_SYSTEM_TYPE, MbrPartitionEntry};

use crate::error::{MisoError, Result};

/// Logical block size for ISO 9660, mandated by ECMA-119.
pub const SECTOR_SIZE: usize = 2048;

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
/// write operation fails.
pub fn write<W: Write, B: FnOnce(&mut W) -> Result<()>>(
    out: &mut W,
    esp_size: u64,
    esp_builder: B,
) -> Result<()> {
    let esp_size_u32 = u32::try_from(esp_size)
        .map_err(|_err| MisoError::Iso("ISO field must fit in u32".to_owned()))?;
    let esp_sectors = el_torito_sector_count(esp_size)?;
    let esp_sectors_u32 =
        u32::try_from(usize::try_from(esp_size).unwrap_or(0).div_ceil(SECTOR_SIZE))
            .unwrap_or(u32::MAX);
    let total_sectors = LBA_FILE_DATA
        .checked_add(esp_sectors_u32)
        .and_then(|sum| sum.checked_add(1))
        .ok_or_else(|| MisoError::Iso("u32 addition for ISO sectors overflowed".to_owned()))?;

    write_system_area(
        out,
        u64::from(LBA_FILE_DATA).saturating_mul(u64::try_from(SECTOR_SIZE).unwrap_or(u64::MAX)),
        esp_size,
    )?;
    out.write_all(&build_pvd(total_sectors, esp_size_u32)?)?;
    out.write_all(&build_boot_record_vd())?;
    out.write_all(&build_vd_terminator())?;
    out.write_all(&build_path_table_l())?;
    out.write_all(&build_path_table_m())?;
    out.write_all(&build_boot_catalog(LBA_FILE_DATA, esp_sectors)?)?;
    out.write_all(&build_root_dir(esp_size_u32)?)?;
    esp_builder(out)?;

    let esp_total = usize::try_from(esp_size).unwrap_or(0);
    let pad = esp_total.rem_euclid(SECTOR_SIZE);
    if pad != 0 {
        let len = SECTOR_SIZE.saturating_sub(pad);
        out.write_all(ZERO_SECTOR.get(..len).unwrap_or(&ZERO_SECTOR))?;
    }

    Ok(())
}

/// A zero-filled ISO 9660 sector used for padding.
const ZERO_SECTOR: [u8; SECTOR_SIZE] = [0; SECTOR_SIZE];

/// Writes 16 sectors of system area (bytes 0–32767) with a hybrid MBR at byte 446.
fn write_system_area<W: Write>(
    writer: &mut W,
    efi_offset_bytes: u64,
    efi_size_bytes: u64,
) -> Result<()> {
    let start_lba = u32::try_from(efi_offset_bytes >> 9)
        .map_err(|_err| MisoError::Iso("EFI image offset sectors must fit in u32".to_owned()))?;
    let size_lba = u32::try_from(efi_size_bytes >> 9)
        .map_err(|_err| MisoError::Iso("EFI image size sectors must fit in u32".to_owned()))?;

    let entry = MbrPartitionEntry {
        bootable: false,
        partition_type: MBR_EFI_SYSTEM_TYPE,
        starting_lba: start_lba,
        size_lba,
    };
    let mut sector = ZERO_SECTOR;
    let mbr = mbr_bytes(&entry);
    if let Some(dst) = sector.get_mut(..mbr.len()) {
        dst.copy_from_slice(&mbr);
    }
    writer.write_all(&sector)?;

    for _ in 1..16 {
        writer.write_all(&ZERO_SECTOR)?;
    }

    Ok(())
}

/// Returns the El Torito boot image sector count, rejecting oversized EFI images.
fn el_torito_sector_count(esp_size: u64) -> Result<u16> {
    let esp_sectors = usize::try_from(esp_size).unwrap_or(0).div_ceil(SECTOR_SIZE);
    if esp_sectors > usize::from(u16::MAX) {
        return Err(MisoError::Iso(format!(
            "EFI boot image too large for El Torito: {esp_sectors} sectors > {}",
            u16::MAX
        )));
    }

    Ok(u16::try_from(esp_sectors).unwrap_or(u16::MAX))
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
    use super::*;
    use crate::error::MisoError;

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
        let oversized = u64::try_from(usize::from(u16::MAX) + 1).unwrap_or(u64::MAX) * 2048;

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
    fn checksum_word_rejects_non_word_sized_chunks() {
        // ARRANGE
        let short_chunk = [0xAA_u8];

        // ACT
        let err = checksum_word(&short_chunk).expect_err("short chunk must fail");

        // ASSERT
        assert!(matches!(err, MisoError::Iso(_)));
        assert!(err.to_string().contains("2 bytes"));
    }
}
