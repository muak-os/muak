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

    let mut gpt = Table::create(out, SECTOR_SIZE, [0xff; 16])?;
    let placement = gpt.place_partition(
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
    )?;
    gpt.write(out)?;

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
        let mut gpt = Table::create(&mut disk, SECTOR_SIZE, [0xff; 16])?;
        match try_layout(
            gpt.place_partition(
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
            )
            .map(|_| ()),
            disk_sectors,
        )? {
            Some(disk_sectors) => return Ok(disk_sectors),
            None => disk_sectors += ALIGN_1_MIB_SECTORS,
        }
    }
}

fn try_layout(
    placement: Result<(), parttable::GptError>,
    disk_sectors: u64,
) -> Result<Option<u64>, MisoError> {
    match placement {
        Ok(()) => Ok(Some(disk_sectors)),
        Err(parttable::GptError::InvalidPlacement(_)) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use esp::{Arch, EspSpec};
    use parttable::{MBR_PROTECTIVE_GPT_TYPE, PlacementRequest, Size, Slot, Start, Table};

    use super::*;

    fn minimal_esp() -> Vec<u8> {
        let spec = EspSpec::with_uki(Arch::X86_64, b"fake-uki".to_vec(), vec![]);
        esp::build(&spec).expect("should build FAT32 image")
    }

    fn try_place_efi_partition(
        disk_sectors: u64,
        efi_image_bytes: u64,
    ) -> Result<(), parttable::GptError> {
        let mut disk = Cursor::new(vec![0u8; (disk_sectors * SECTOR_SIZE) as usize]);
        let mut gpt = Table::create(&mut disk, SECTOR_SIZE, [0xff; 16])?;

        gpt.place_partition(
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
        )?;

        Ok(())
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

    #[test]
    fn layout_disk_grows_until_the_esp_fits() {
        // ARRANGE
        let efi_image_bytes = 3 * 1024 * 1024;

        // ACT
        let disk_sectors = layout_disk(efi_image_bytes).expect("layout_disk must succeed");
        let previous_attempt =
            try_place_efi_partition(disk_sectors - ALIGN_1_MIB_SECTORS, efi_image_bytes);
        let successful_attempt = try_place_efi_partition(disk_sectors, efi_image_bytes);

        // ASSERT
        assert!(
            matches!(
                previous_attempt,
                Err(parttable::GptError::InvalidPlacement(_))
            ),
            "previous disk size must be too small to fit the ESP"
        );
        successful_attempt.expect("returned disk size must fit the ESP");
        assert!(disk_sectors > ALIGN_1_MIB_SECTORS * 2);
        assert_eq!(disk_sectors % ALIGN_1_MIB_SECTORS, 0);
    }

    #[test]
    fn layout_result_returns_disk_size_when_partition_fits() {
        // ARRANGE
        let disk_sectors = ALIGN_1_MIB_SECTORS * 4;

        // ACT
        let result = try_layout(Ok(()), disk_sectors).expect("successful placement must work");

        // ASSERT
        assert_eq!(result, Some(disk_sectors));
    }

    #[test]
    fn layout_result_retries_when_partition_does_not_fit() {
        // ARRANGE
        let disk_sectors = ALIGN_1_MIB_SECTORS * 2;

        // ACT
        let result = try_layout(
            Err(parttable::GptError::InvalidPlacement(
                "partition does not fit".to_owned(),
            )),
            disk_sectors,
        )
        .expect("invalid placement should trigger a retry");

        // ASSERT
        assert_eq!(result, None);
    }

    #[test]
    fn layout_result_propagates_unexpected_gpt_errors() {
        // ARRANGE
        let disk_sectors = ALIGN_1_MIB_SECTORS * 2;

        // ACT
        let err = try_layout(
            Err(parttable::GptError::Gpt("corrupt header".to_owned())),
            disk_sectors,
        )
        .expect_err("unexpected GPT errors must be propagated");

        // ASSERT
        assert!(matches!(err, MisoError::Gpt(_)));
        assert!(err.to_string().contains("corrupt header"));
    }
}
