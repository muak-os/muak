//! Raw disk image writer with protective MBR + GPT + EFI System Partition.

use std::io::{Read, Write};

use ::esp::builder::Layout;
use parttable::error::ParttableError;
use parttable::error::Result as ParttableResult;
use parttable::gpt::table::Table;
use parttable::gpt::types::{ALIGN_1_MIB_SECTORS, EFI_GUID, PlacementRequest, Size, Slot, Start};

use crate::error::{MisoError, Result};
use crate::esp;

const SECTOR_SIZE: u64 = 512;

type PlacementResult = ParttableResult<()>;

/// Builds a raw GPT disk image containing the ESP into any `Write` sink.
///
/// When `compression_level` is `Some`, the output is transparently compressed
/// with zstd at the given level.
///
/// # Errors
///
/// Returns an error if ESP construction fails, compression level validation fails,
/// raw image creation fails, or output writing/compression fails.
pub fn build<'data, 'ctx, W: Write>(
    layout: &'ctx Layout<'data>,
    readers: &mut [&'data mut (dyn Read + 'data)],
    out: &mut W,
    compression_level: Option<i32>,
) -> Result<()> {
    if let Some(level) = compression_level {
        let level = validate_compression_level(level)?;
        let mut encoder = zstd::Encoder::new(out, level).map_err(MisoError::ZstdInit)?;
        write(&mut encoder, layout.total_size, |w| {
            esp::build(layout, readers, w)
        })?;
        encoder.finish().map_err(MisoError::Compression)?;
    } else {
        write(out, layout.total_size, |w| esp::build(layout, readers, w))?;
    }

    Ok(())
}

fn write<W: Write, B: FnOnce(&mut W) -> Result<()>>(
    out: &mut W,
    esp_size: u64,
    esp_builder: B,
) -> Result<()> {
    let disk_sectors = layout_disk(esp_size)?;

    let mut table = Table::create(disk_sectors, SECTOR_SIZE, [0xff; 16])?;
    let request = PlacementRequest {
        slot: Slot::Exact(1),
        start: Start::FirstUsable,
        size: Size::Bytes(esp_size),
        alignment_lba: ALIGN_1_MIB_SECTORS,
        type_guid: EFI_GUID,
        unique_guid: [0xAA; 16],
        attributes: 0,
        name: "EFI".to_owned(),
    };
    let placement = table.place_partition(request, SECTOR_SIZE)?;

    let partition_offset = placement
        .partition
        .starting_lba
        .checked_mul(SECTOR_SIZE)
        .ok_or(MisoError::Gpt("raw image arithmetic overflowed".to_owned()))?;

    table.write_primary_to(disk_sectors, out)?;
    write_zeros(
        out,
        partition_offset.saturating_sub(table.primary_gpt_size()),
    )?;
    esp_builder(out)?;
    write_zeros(
        out,
        table
            .backup_data_offset(disk_sectors)
            .saturating_sub(partition_offset.saturating_add(esp_size)),
    )?;
    table.write_backup_to(disk_sectors, out)?;

    Ok(())
}

fn write_zeros<W: Write>(writer: &mut W, count: u64) -> Result<()> {
    const BUF: &[u8] = &[0; 4096];
    let mut remaining = count;
    while remaining > 0 {
        let chunk = usize::try_from(remaining.min(4096)).unwrap_or(4096);
        let data = BUF.get(..chunk).unwrap_or(BUF);
        writer.write_all(data)?;
        remaining = remaining.saturating_sub(u64::try_from(chunk).unwrap_or(0));
    }

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

fn validate_compression_level(level: i32) -> Result<i32> {
    let range = zstd::compression_level_range();

    if level == 0 || range.contains(&level) {
        Ok(level)
    } else {
        Err(MisoError::InvalidCompressionLevel {
            level,
            min: *range.start(),
            max: *range.end(),
        })
    }
}

#[cfg(test)]
mod tests {
    use parttable::gpt::{
        table::Table,
        types::{ALIGN_1_MIB_SECTORS, EFI_GUID, PlacementRequest, Size, Slot, Start},
    };

    use super::*;
    use crate::error::MisoError;

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
