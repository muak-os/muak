//! Raw disk image writer with protective MBR + GPT + EFI System Partition.

use std::io::{Read, Seek, SeekFrom, Write};

use gptman::{GPT, GPTPartitionEntry};
use parttable::{
    ALIGN_1_MIB_SECTORS, EFI_GUID, align_up_lba, write_gpt_protective_mbr,
};

use crate::MisoError;

/// Sector size for the raw disk image (512 bytes).
const SECTOR_SIZE: u64 = 512;

/// Writes a raw GPT disk image containing the FAT32 ESP into `out`.
pub fn write<W: Write + Read + Seek>(out: &mut W, efi_image: &[u8]) -> Result<(), MisoError> {
    let esp_sectors = efi_image.len().div_ceil(SECTOR_SIZE as usize) as u64;
    let gpt_overhead_sectors = 34;
    let esp_start = align_up_lba(gpt_overhead_sectors, ALIGN_1_MIB_SECTORS);
    let esp_end = esp_start + esp_sectors - 1;
    let disk_sectors =
        align_up_lba(esp_end + 1 + gpt_overhead_sectors, ALIGN_1_MIB_SECTORS) + ALIGN_1_MIB_SECTORS;
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

    write_gpt_protective_mbr(out, disk_size, SECTOR_SIZE)?;

    out.seek(SeekFrom::Start(esp_start * SECTOR_SIZE))?;
    out.write_all(efi_image)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use esp::{Arch, EspSpec};
    use parttable::MBR_PROTECTIVE_GPT_TYPE;

    use super::*;

    fn minimal_esp() -> Vec<u8> {
        let spec = EspSpec::with_uki(Arch::X86_64, b"fake-uki".to_vec(), vec![]);
        esp::build(&spec).expect("should build FAT32 image")
    }

    #[test]
    fn write_raw_produces_valid_gpt() {
        // ARRANGE
        let esp = minimal_esp();
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write(&mut buf, &esp).expect("write_raw must succeed");

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
    fn write_raw_has_protective_mbr() {
        // ARRANGE
        let esp = minimal_esp();
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write(&mut buf, &esp).expect("write_raw must succeed");

        // ASSERT
        let data = buf.into_inner();
        assert_eq!(data[510], 0x55);
        assert_eq!(data[511], 0xAA);
        assert_eq!(
            data[450], MBR_PROTECTIVE_GPT_TYPE,
            "protective MBR type must be 0xEE"
        );
    }

    #[test]
    fn write_raw_esp_partition_has_efi_guid() {
        // ARRANGE
        let esp = minimal_esp();
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write(&mut buf, &esp).expect("write_raw must succeed");

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
    fn write_raw_esp_contains_uki_data() {
        // ARRANGE
        let spec = EspSpec::with_uki(Arch::X86_64, b"test-uki-content".to_vec(), vec![]);
        let esp = esp::build(&spec).expect("build FAT32");
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write(&mut buf, &esp).expect("write_raw must succeed");

        // ASSERT
        let data = buf.into_inner();
        let mut cursor = Cursor::new(&data);
        let gpt = GPT::find_from(&mut cursor).expect("GPT must be valid");
        let (_, part) = gpt
            .iter()
            .find(|(_, p)| p.is_used())
            .expect("must have partition");
        let offset = (part.starting_lba * SECTOR_SIZE) as usize;
        let esp_data = &data[offset..offset + esp.len()];
        assert_eq!(esp_data, esp.as_slice());
    }

    #[test]
    fn write_raw_partition_is_aligned() {
        // ARRANGE
        let esp = minimal_esp();
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write(&mut buf, &esp).expect("write_raw must succeed");

        // ASSERT
        let mut cursor = Cursor::new(buf.into_inner());
        let gpt = GPT::find_from(&mut cursor).expect("GPT must be valid");
        let (_, part) = gpt
            .iter()
            .find(|(_, p)| p.is_used())
            .expect("must have partition");
        assert_eq!(
            part.starting_lba % ALIGN_1_MIB_SECTORS,
            0,
            "ESP start must be aligned to 1 MiB"
        );
    }

    #[test]
    fn write_raw_disk_size_is_sector_aligned() {
        // ARRANGE
        let esp = minimal_esp();
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write(&mut buf, &esp).expect("write_raw must succeed");

        // ASSERT
        let data = buf.into_inner();
        assert_eq!(
            data.len() as u64 % SECTOR_SIZE,
            0,
            "disk image must be sector-aligned"
        );
    }

    #[test]
    fn write_raw_partition_name_is_efi() {
        // ARRANGE
        let esp = minimal_esp();
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write(&mut buf, &esp).expect("write_raw must succeed");

        // ASSERT
        let mut cursor = Cursor::new(buf.into_inner());
        let gpt = GPT::find_from(&mut cursor).expect("GPT must be valid");
        let (_, part) = gpt
            .iter()
            .find(|(_, p)| p.is_used())
            .expect("must have partition");
        assert_eq!(part.partition_name.as_str(), "EFI");
    }
}
