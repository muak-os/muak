//! Raw disk image writer with protective MBR + GPT + EFI System Partition.

use std::io::{Read, Seek, SeekFrom, Write};

use gptman::{GPT, GPTPartitionEntry};

use crate::MisoError;

/// Sector size for the raw disk image (512 bytes).
const SECTOR_SIZE: u64 = 512;

/// Partition alignment in sectors (1 MiB boundary).
const ALIGN_SECTORS: u64 = 2048;

/// EFI System Partition type GUID (C12A7328-F81F-11D2-BA4B-00A0C93EC93B), mixed-endian.
const EFI_GUID: [u8; 16] = [
    0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b,
];

/// Rounds `lba` up to the nearest multiple of `align`.
fn align_up(lba: u64, align: u64) -> u64 {
    if lba.is_multiple_of(align) {
        lba
    } else {
        lba + (align - (lba % align))
    }
}

/// Writes a protective MBR covering the entire disk.
fn write_protective_mbr<W: Write + Seek>(w: &mut W, disk_size: u64) -> Result<(), MisoError> {
    let mut pmbr = [0u8; 512];
    pmbr[510] = 0x55;
    pmbr[511] = 0xAA;
    pmbr[446] = 0x00;
    pmbr[450] = 0xEE;
    pmbr[454] = 0x01;

    let total_lbas = disk_size / SECTOR_SIZE;
    let part_size = total_lbas.saturating_sub(1).min(u32::MAX as u64) as u32;
    pmbr[458..462].copy_from_slice(&part_size.to_le_bytes());

    w.seek(SeekFrom::Start(0))?;
    w.write_all(&pmbr)?;
    Ok(())
}

/// Writes a raw GPT disk image containing the FAT32 ESP into `out`.
pub fn write_img<W: Write + Read + Seek>(out: &mut W, efi_image: &[u8]) -> Result<(), MisoError> {
    let esp_sectors = efi_image.len().div_ceil(SECTOR_SIZE as usize) as u64;
    let gpt_overhead_sectors = 34;
    let esp_start = align_up(gpt_overhead_sectors, ALIGN_SECTORS);
    let esp_end = esp_start + esp_sectors - 1;
    let disk_sectors = align_up(esp_end + 1 + gpt_overhead_sectors, ALIGN_SECTORS) + ALIGN_SECTORS;
    let disk_size = disk_sectors * SECTOR_SIZE;

    let zeroed = vec![0u8; disk_size as usize];
    out.seek(SeekFrom::Start(0))?;
    out.write_all(&zeroed)?;

    let mut gpt =
        GPT::new_from(out, SECTOR_SIZE, [0xff; 16]).map_err(|e| MisoError::Gpt(e.to_string()))?;

    gpt[1] = GPTPartitionEntry {
        partition_type_guid: EFI_GUID,
        unique_partition_guid: [0xAA; 16],
        starting_lba: esp_start,
        ending_lba: esp_end,
        attribute_bits: 0,
        partition_name: "EFI".into(),
    };

    gpt.write_into(out)
        .map_err(|e| MisoError::Gpt(e.to_string()))?;

    write_protective_mbr(out, disk_size)?;

    out.seek(SeekFrom::Start(esp_start * SECTOR_SIZE))?;
    out.write_all(efi_image)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::fat::build_efi_image;

    fn minimal_esp() -> Vec<u8> {
        build_efi_image(b"fake-uki", "BOOTX64.EFI").expect("should build FAT32 image")
    }

    #[test]
    fn write_img_produces_valid_gpt() {
        // ARRANGE
        let esp = minimal_esp();
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write_img(&mut buf, &esp).expect("write_img must succeed");

        // ASSERT
        let data = buf.into_inner();
        assert!(!data.is_empty());
        let mut cursor = Cursor::new(data);
        let gpt = GPT::find_from(&mut cursor).expect("GPT must be valid");
        assert!(
            gpt.iter().any(|(_, p)| p.is_used()),
            "must have a partition"
        );
    }

    #[test]
    fn write_img_has_protective_mbr() {
        // ARRANGE
        let esp = minimal_esp();
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write_img(&mut buf, &esp).expect("write_img must succeed");

        // ASSERT
        let data = buf.into_inner();
        assert_eq!(data[510], 0x55);
        assert_eq!(data[511], 0xAA);
        assert_eq!(data[450], 0xEE, "protective MBR type must be 0xEE");
    }

    #[test]
    fn write_img_esp_partition_has_efi_guid() {
        // ARRANGE
        let esp = minimal_esp();
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write_img(&mut buf, &esp).expect("write_img must succeed");

        // ASSERT
        let mut cursor = Cursor::new(buf.into_inner());
        let gpt = GPT::find_from(&mut cursor).expect("GPT must be valid");
        let (_, part) = gpt
            .iter()
            .find(|(_, p)| p.is_used())
            .expect("must have partition");
        assert_eq!(part.partition_type_guid, EFI_GUID);
    }

    #[test]
    fn write_img_esp_contains_uki_data() {
        // ARRANGE
        let uki = b"test-uki-content";
        let esp = build_efi_image(uki, "BOOTX64.EFI").expect("build FAT32");
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write_img(&mut buf, &esp).expect("write_img must succeed");

        // ASSERT
        let data = buf.into_inner();
        let mut cursor = Cursor::new(&data);
        let gpt = GPT::find_from(&mut cursor).expect("GPT must be valid");
        let (_, part) = gpt
            .iter()
            .find(|(_, p)| p.is_used())
            .expect("must have partition");
        let offset = (part.starting_lba * SECTOR_SIZE) as usize;
        let fat_data = &data[offset..offset + esp.len()];
        let mut fat_cursor = Cursor::new(fat_data.to_vec());
        let fs = fatfs::FileSystem::new(&mut fat_cursor, fatfs::FsOptions::new())
            .expect("FAT32 must be valid");
        let mut file = fs
            .root_dir()
            .open_dir("EFI")
            .expect("EFI dir")
            .open_dir("BOOT")
            .expect("BOOT dir")
            .open_file("BOOTX64.EFI")
            .expect("UKI file");
        let mut content = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut content).expect("read UKI");
        assert_eq!(content, uki);
    }

    #[test]
    fn write_img_partition_is_aligned() {
        // ARRANGE
        let esp = minimal_esp();
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write_img(&mut buf, &esp).expect("write_img must succeed");

        // ASSERT
        let mut cursor = Cursor::new(buf.into_inner());
        let gpt = GPT::find_from(&mut cursor).expect("GPT must be valid");
        let (_, part) = gpt
            .iter()
            .find(|(_, p)| p.is_used())
            .expect("must have partition");
        assert_eq!(
            part.starting_lba % ALIGN_SECTORS,
            0,
            "ESP start must be aligned to 1 MiB"
        );
    }

    #[test]
    fn write_img_disk_size_is_sector_aligned() {
        // ARRANGE
        let esp = minimal_esp();
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write_img(&mut buf, &esp).expect("write_img must succeed");

        // ASSERT
        let data = buf.into_inner();
        assert_eq!(
            data.len() as u64 % SECTOR_SIZE,
            0,
            "disk image must be sector-aligned"
        );
    }

    #[test]
    fn write_img_partition_name_is_efi() {
        // ARRANGE
        let esp = minimal_esp();
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write_img(&mut buf, &esp).expect("write_img must succeed");

        // ASSERT
        let mut cursor = Cursor::new(buf.into_inner());
        let gpt = GPT::find_from(&mut cursor).expect("GPT must be valid");
        let (_, part) = gpt
            .iter()
            .find(|(_, p)| p.is_used())
            .expect("must have partition");
        assert_eq!(part.partition_name.as_str(), "EFI");
    }

    #[test]
    fn align_up_already_aligned() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(align_up(2048, 2048), 2048);
    }

    #[test]
    fn align_up_rounds_unaligned() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(align_up(2049, 2048), 4096);
        assert_eq!(align_up(1, 2048), 2048);
    }

    #[test]
    fn align_up_zero() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(align_up(0, 2048), 0);
    }
}
