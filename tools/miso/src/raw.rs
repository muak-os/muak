//! Raw disk image writer with protective MBR + GPT + EFI System Partition.

use std::io::{Read, Seek, SeekFrom, Write};

use parttable::{ALIGN_1_MIB_SECTORS, EFI_GUID, PlacementRequest, Size, Slot, Start, Table};

use crate::MisoError;

/// Sector size for the raw disk image (512 bytes).
const SECTOR_SIZE: u64 = 512;

/// Writes a raw GPT disk image containing the FAT32 ESP into `out`.
pub fn write<W: Write + Read + Seek>(out: &mut W, efi_image: &[u8]) -> Result<(), MisoError> {
    let disk_sectors = layout_disk(efi_image.len() as u64)?;
    let disk_size = disk_sectors * SECTOR_SIZE;

    let zeroed = vec![0u8; disk_size as usize];
    out.seek(SeekFrom::Start(0))?;
    out.write_all(&zeroed)?;

    let mut gpt = Table::create(out, SECTOR_SIZE, [0xff; 16])
        .map_err(|err| MisoError::Gpt(err.to_string()))?;
    let placement = gpt
        .place_partition(
            PlacementRequest {
                slot: Slot::Exact(1),
                start: Start::FirstUsable,
                size: Size::Bytes(efi_image.len() as u64),
                alignment_lba: ALIGN_1_MIB_SECTORS,
                type_guid: EFI_GUID,
                unique_guid: [0xAA; 16],
                attributes: 0,
                name: "EFI".to_owned(),
            },
            SECTOR_SIZE,
        )
        .map_err(|err| MisoError::Gpt(err.to_string()))?;
    gpt.write(out)
        .map_err(|err| MisoError::Gpt(err.to_string()))?;

    parttable::write_gpt_protective_mbr(out, disk_size, SECTOR_SIZE)?;

    out.seek(SeekFrom::Start(
        placement.partition.starting_lba * SECTOR_SIZE,
    ))?;
    out.write_all(efi_image)?;

    Ok(())
}

fn layout_disk(efi_image_bytes: u64) -> Result<u64, MisoError> {
    let mut disk_sectors = ALIGN_1_MIB_SECTORS * 2;

    loop {
        let mut disk = std::io::Cursor::new(vec![0u8; (disk_sectors * SECTOR_SIZE) as usize]);
        let mut gpt = Table::create(&mut disk, SECTOR_SIZE, [0xff; 16])
            .map_err(|err| MisoError::Gpt(err.to_string()))?;
        match gpt.place_partition(
            PlacementRequest {
                slot: Slot::Exact(1),
                start: Start::FirstUsable,
                size: Size::Bytes(efi_image_bytes),
                alignment_lba: ALIGN_1_MIB_SECTORS,
                type_guid: EFI_GUID,
                unique_guid: [0xAA; 16],
                attributes: 0,
                name: "EFI".to_owned(),
            },
            SECTOR_SIZE,
        ) {
            Ok(_) => return Ok(disk_sectors),
            Err(parttable::GptError::InvalidPlacement(_)) => {
                disk_sectors += ALIGN_1_MIB_SECTORS;
            }
            Err(err) => return Err(MisoError::Gpt(err.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use esp::{Arch, EspSpec};
    use parttable::{MBR_PROTECTIVE_GPT_TYPE, Table};

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
        let gpt = Table::read(&mut cursor).expect("GPT must be valid");
        assert!(gpt.has_used_partitions(), "must have a partition");
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
        let gpt = Table::read(&mut cursor).expect("GPT must be valid");
        let part = gpt.partition(1).expect("must have partition");
        assert_eq!(part.type_guid, EFI_GUID);
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
        let gpt = Table::read(&mut cursor).expect("GPT must be valid");
        let part = gpt.partition(1).expect("must have partition");
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
        let gpt = Table::read(&mut cursor).expect("GPT must be valid");
        let part = gpt.partition(1).expect("must have partition");
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
        let gpt = Table::read(&mut cursor).expect("GPT must be valid");
        let part = gpt.partition(1).expect("must have partition");
        assert_eq!(part.name.as_str(), "EFI");
    }
}
