//! Raw disk image writer with protective MBR + GPT + EFI System Partition.

use std::io::{Cursor, Seek as _, SeekFrom, Write};

use parttable::error::ParttableError;
use parttable::error::Result as ParttableResult;
use parttable::gpt::table::Table;
use parttable::gpt::types::{ALIGN_1_MIB_SECTORS, EFI_GUID, PlacementRequest, Size, Slot, Start};

use crate::error::{MisoError, Result};

const SECTOR_SIZE: u64 = 512;

type PlacementResult = ParttableResult<()>;

/// Writes a raw GPT disk image containing the FAT32 ESP into any `Write` sink.
///
/// # Errors
///
/// Returns an error if the image layout overflows, GPT/MBR generation fails,
/// or any write operation fails.
pub fn write<W: Write>(out: &mut W, efi_image: &[u8]) -> Result<()> {
    let efi_image_len = u64::try_from(efi_image.len()).unwrap_or(u64::MAX);
    let disk_sectors = layout_disk(efi_image_len)?;
    let disk_size = disk_sectors
        .checked_mul(SECTOR_SIZE)
        .ok_or(MisoError::Gpt("raw image arithmetic overflowed".to_owned()))?;

    let mut table = Table::create(disk_sectors, SECTOR_SIZE, [0xff; 16])?;
    let request = PlacementRequest {
        slot: Slot::Exact(1),
        start: Start::FirstUsable,
        size: Size::Bytes(efi_image_len),
        alignment_lba: ALIGN_1_MIB_SECTORS,
        type_guid: EFI_GUID,
        unique_guid: [0xAA; 16],
        attributes: 0,
        name: "EFI".to_owned(),
    };
    let placement = table.place_partition(request, SECTOR_SIZE)?;

    let mut cursor = {
        let mut buf = Cursor::new(Vec::new());
        let last_byte = disk_size
            .checked_sub(1)
            .ok_or(MisoError::Gpt("raw image size underflowed".to_owned()))?;
        drop(buf.seek(SeekFrom::Start(last_byte)));
        drop(buf.write_all(&[0]));
        drop(buf.seek(SeekFrom::Start(0)));

        buf
    };

    table.write_primary_to(disk_sectors, &mut cursor)?;

    let partition_offset = placement
        .partition
        .starting_lba
        .checked_mul(SECTOR_SIZE)
        .ok_or(MisoError::Gpt("raw image arithmetic overflowed".to_owned()))?;
    cursor.seek(SeekFrom::Start(partition_offset))?;
    cursor.write_all(efi_image)?;

    table.write_backup_to(disk_sectors, &mut cursor)?;

    out.write_all(cursor.into_inner().as_slice())?;

    Ok(())
}

fn layout_disk(efi_image_bytes: u64) -> Result<u64> {
    let mut disk_sectors = ALIGN_1_MIB_SECTORS * 2;

    loop {
        let mut gpt = Table::create(disk_sectors, SECTOR_SIZE, [0xff; 16])?;
        let request = PlacementRequest {
            slot: Slot::Exact(1),
            start: Start::FirstUsable,
            size: Size::Bytes(efi_image_bytes),
            alignment_lba: ALIGN_1_MIB_SECTORS,
            type_guid: EFI_GUID,
            unique_guid: [0xAA; 16],
            attributes: 0,
            name: "EFI".to_owned(),
        };
        let placement = gpt.place_partition(request, SECTOR_SIZE).map(|_| ());

        match try_layout(placement, disk_sectors)? {
            Some(disk_sectors) => return Ok(disk_sectors),
            None => {
                disk_sectors =
                    disk_sectors
                        .checked_add(ALIGN_1_MIB_SECTORS)
                        .ok_or(MisoError::Gpt(
                            "raw disk sector count overflowed".to_owned(),
                        ))?;
            }
        }
    }
}

fn try_layout(placement: PlacementResult, disk_sectors: u64) -> Result<Option<u64>> {
    match placement {
        Ok(()) => Ok(Some(disk_sectors)),
        Err(ParttableError::InvalidPlacement(_)) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use esp::{Arch, EspSpec, build};
    use parttable::{
        gpt::{
            table::Table as GptTable,
            types::{ALIGN_1_MIB_SECTORS, EFI_GUID, PlacementRequest, Size, Slot, Start},
        },
        mbr::types::MBR_PROTECTIVE_GPT_TYPE,
    };

    use super::*;
    use crate::error::MisoError;

    fn minimal_esp() -> Vec<u8> {
        let spec = EspSpec::with_uki(Arch::X86_64, b"fake-uki".to_vec(), vec![]);
        build(&spec).expect("should build FAT32 image")
    }

    fn try_place_efi_partition(disk_sectors: u64, efi_image_bytes: u64) -> Result<()> {
        let mut gpt = Table::create(disk_sectors, SECTOR_SIZE, [0xff; 16])?;

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
        let mut cursor = Cursor::new(data.clone());
        let gpt = GptTable::read(&mut cursor).expect("GPT must be readable");
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
        assert_eq!(
            data.get(510).copied().expect("MBR byte 510 must exist"),
            0x55
        );
        assert_eq!(
            data.get(511).copied().expect("MBR byte 511 must exist"),
            0xAA
        );
        assert_eq!(
            data.get(450)
                .copied()
                .expect("protective MBR type byte must exist"),
            MBR_PROTECTIVE_GPT_TYPE,
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
        let data = buf.into_inner();
        let mut cursor = Cursor::new(data.clone());
        let gpt = GptTable::read(&mut cursor).expect("must read GPT");
        let part = gpt.partition(1).expect("must have partition");
        assert_eq!(part.type_guid, EFI_GUID);
    }

    #[test]
    fn write_raw_esp_contains_uki_data() {
        // ARRANGE
        let spec = EspSpec::with_uki(Arch::X86_64, b"test-uki-content".to_vec(), vec![]);
        let esp = build(&spec).expect("build FAT32");
        let mut buf = Cursor::new(Vec::new());

        // ACT
        write(&mut buf, &esp).expect("write_raw must succeed");

        // ASSERT
        let data = buf.into_inner();
        let mut cursor = Cursor::new(data.clone());
        let gpt = GptTable::read(&mut cursor).expect("must read GPT");
        let part = gpt.partition(1).expect("must have partition");
        let offset = usize::try_from(
            part.starting_lba
                .checked_mul(SECTOR_SIZE)
                .expect("partition offset must fit in u64"),
        )
        .expect("partition offset must fit in usize");
        let esp_data = data
            .get(offset..offset + esp.len())
            .expect("ESP data range must exist");
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
        let data = buf.into_inner();
        let mut cursor = Cursor::new(data.clone());
        let gpt = GptTable::read(&mut cursor).expect("must read GPT");
        let part = gpt.partition(1).expect("must have partition");
        assert_eq!(
            part.starting_lba.rem_euclid(ALIGN_1_MIB_SECTORS),
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
            u64::try_from(data.len())
                .expect("disk image length must fit in u64")
                .rem_euclid(SECTOR_SIZE),
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
        let data = buf.into_inner();
        let mut cursor = Cursor::new(data.clone());
        let gpt = GptTable::read(&mut cursor).expect("must read GPT");
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
            matches!(previous_attempt, Err(MisoError::Gpt(_))),
            "previous disk size must be too small to fit the ESP"
        );
        successful_attempt.expect("returned disk size must fit the ESP");
        assert!(disk_sectors > ALIGN_1_MIB_SECTORS * 2);
        assert_eq!(disk_sectors.rem_euclid(ALIGN_1_MIB_SECTORS), 0);
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
            Err(ParttableError::InvalidPlacement(
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
            Err(ParttableError::Gpt("corrupt header".to_owned())),
            disk_sectors,
        )
        .expect_err("unexpected GPT errors must be propagated");

        // ASSERT
        assert!(matches!(err, MisoError::Gpt(_)));
        assert!(err.to_string().contains("corrupt header"));
    }
}
