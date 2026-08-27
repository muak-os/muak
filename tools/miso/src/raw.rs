//! Raw disk image writer with protective MBR + GPT + EFI System Partition.

use std::io::{Read, Write};

use esp::EFI_GUID;
use esp::image;
use esp::layout::Layout;
use parttable::error::ParttableError;
use parttable::error::Result as PlacementResult;
use parttable::gpt;
use parttable::gpt::layout::{ALIGN_1_MIB_SECTORS, PlacementRequest, Size, Slot, Start};
use parttable::gpt::table::Table;

use crate::error::{MisoError, Result};

const SECTOR_SIZE: u64 = 512;
const ALIGN_1_MIB_BYTES: u64 = ALIGN_1_MIB_SECTORS.saturating_mul(SECTOR_SIZE);

/// A raw boot blob written at a fixed byte offset before the partition table.
pub struct Blob<'a> {
    /// Byte offset on the boot device where the blob is written.
    pub offset: u64,
    /// Size of the blob payload in bytes.
    pub size: u64,
    /// Streaming reader for the blob payload.
    pub reader: &'a mut dyn Read,
}

/// Builds a raw GPT disk image containing the ESP into any `Write` sink.
///
/// # Errors
///
/// Returns an error if blob placement overlaps the GPT or another blob, ESP
/// construction fails, compression level validation fails, raw image creation
/// fails, or output writing/compression fails.
pub fn build<'data, 'ctx, W: Write>(
    layout: &'ctx Layout<'data>,
    esp: &mut [&'data mut (dyn Read + 'data)],
    blobs: &mut [Blob<'data>],
    partition_start: u64,
    out: &mut W,
    compression_level: Option<i32>,
) -> Result<()> {
    let partition_start = if partition_start == 0 {
        ALIGN_1_MIB_BYTES
    } else {
        partition_start
    };
    validate_blobs(blobs, partition_start)?;
    let esp_size = layout.total_size;

    if let Some(level) = compression_level {
        let level = validate_compression_level(level)?;
        let mut encoder = zstd::Encoder::new(out, level).map_err(MisoError::ZstdInit)?;
        write(&mut encoder, esp_size, partition_start, blobs, |w| {
            image::build(layout, esp, w).map_err(MisoError::Esp)
        })?;
        encoder.finish().map_err(MisoError::Compression)?;
    } else {
        write(out, esp_size, partition_start, blobs, |w| {
            image::build(layout, esp, w).map_err(MisoError::Esp)
        })?;
    }

    Ok(())
}

fn write<W: Write, B: FnOnce(&mut W) -> Result<()>>(
    out: &mut W,
    esp_size: u64,
    partition_start: u64,
    blobs: &mut [Blob],
    esp_builder: B,
) -> Result<()> {
    let disk_sectors = layout_disk(esp_size, partition_start)?;

    let mut table = Table::create(disk_sectors, SECTOR_SIZE, [0xff; 16])?;
    let start_lba = partition_start.div_ceil(SECTOR_SIZE);
    let request = PlacementRequest {
        slot: Slot::Exact(1),
        start: Start::AtOrAfter(start_lba),
        size: Size::Bytes(esp_size),
        alignment_lba: ALIGN_1_MIB_SECTORS,
        type_guid: EFI_GUID,
        unique_guid: [0xAA; 16],
        attributes: 0,
        name: "EFI".to_owned(),
    };
    let placement = request.place(&mut table, SECTOR_SIZE)?;
    let partition_offset = placement
        .partition
        .starting_lba
        .checked_mul(SECTOR_SIZE)
        .ok_or(MisoError::Gpt("raw image arithmetic overflowed".to_owned()))?;

    gpt::io::write_primary(&table, disk_sectors, out)?;
    let mut pos = table.primary_gpt_size();

    blobs.sort_by_key(|blob| blob.offset);
    for blob in blobs.iter_mut() {
        write_zeros(out, blob.offset.saturating_sub(pos))?;
        let written = std::io::copy(&mut blob.reader, out)?;
        pos = blob.offset.saturating_add(written);
    }

    write_zeros(out, partition_offset.saturating_sub(pos))?;
    esp_builder(out)?;
    pos = partition_offset.saturating_add(esp_size);
    let backup_offset = table.backup_data_offset(disk_sectors);
    write_zeros(out, backup_offset.saturating_sub(pos))?;
    gpt::io::write_backup(&table, disk_sectors, out)?;

    Ok(())
}

fn validate_blobs(blobs: &[Blob], partition_start: u64) -> Result<()> {
    let gpt = Table::create(ALIGN_1_MIB_SECTORS * 2, SECTOR_SIZE, [0xff; 16])?;
    let gpt_end = gpt.primary_gpt_size();
    let mut prev_end = 0_u64;
    let mut sorted: Vec<&Blob> = blobs.iter().collect();
    sorted.sort_by_key(|blob| blob.offset);

    for blob in sorted {
        if blob.offset < gpt_end {
            return Err(MisoError::Gpt(format!(
                "blob at offset {} overlaps the primary GPT region",
                blob.offset
            )));
        }
        if blob.offset.saturating_add(blob.size) > partition_start {
            return Err(MisoError::Gpt(format!(
                "blob at offset {} extends past the partition table start {partition_start}",
                blob.offset
            )));
        }
        if blob.offset < prev_end {
            return Err(MisoError::Gpt(format!(
                "blob at offset {} overlaps an earlier blob",
                blob.offset
            )));
        }
        prev_end = blob.offset.saturating_add(blob.size);
    }

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

fn layout_disk(efi_image_bytes: u64, partition_start: u64) -> Result<u64> {
    let start_lba = partition_start.div_ceil(SECTOR_SIZE);
    let mut disk_sectors = ALIGN_1_MIB_SECTORS * 2;

    loop {
        let mut gpt = Table::create(disk_sectors, SECTOR_SIZE, [0xff; 16])?;
        let request = PlacementRequest {
            slot: Slot::Exact(1),
            start: Start::AtOrAfter(start_lba),
            size: Size::Bytes(efi_image_bytes),
            alignment_lba: ALIGN_1_MIB_SECTORS,
            type_guid: EFI_GUID,
            unique_guid: [0xAA; 16],
            attributes: 0,
            name: "EFI".to_owned(),
        };
        let placement = request.place(&mut gpt, SECTOR_SIZE).map(|_| ());

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

fn try_layout(placement: PlacementResult<()>, disk_sectors: u64) -> Result<Option<u64>> {
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
    use esp::EFI_GUID;
    use parttable::gpt::layout::{ALIGN_1_MIB_SECTORS, PlacementRequest, Size, Slot, Start};
    use parttable::gpt::table::Table;

    use super::*;
    use crate::error::MisoError;

    fn try_place_efi_partition(disk_sectors: u64, efi_image_bytes: u64) -> Result<()> {
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

        request.place(&mut gpt, SECTOR_SIZE)?;

        Ok(())
    }

    #[test]
    fn layout_disk_grows_until_the_esp_fits() {
        // ARRANGE
        let efi_image_bytes = 3 * 1024 * 1024;

        // ACT
        let disk_sectors =
            layout_disk(efi_image_bytes, ALIGN_1_MIB_BYTES).expect("layout_disk must succeed");
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

    #[test]
    fn raw_blob_written_at_offset_and_partition_shifted() {
        // ARRANGE
        use std::io::Cursor;

        use esp::FileMeta;
        use esp::arch::Arch;
        use esp::layout::compute;
        use parttable::gpt::io;

        let uki = vec![0xCC_u8; 1024];
        let uki_size = u64::try_from(uki.len()).unwrap_or(0);
        let mut uki_cursor = Cursor::new(uki);

        let blob = vec![0xAB_u8; 1024];
        let blob_offset = 32 * 1024;
        let mut blob_cursor = Cursor::new(blob.clone());

        let files = &[FileMeta::new(Arch::X86_64.boot_path(), uki_size)];
        let layout = compute(files).expect("compute layout");

        let mut out = Cursor::new(Vec::new());
        let mut readers: Vec<&mut dyn Read> = vec![&mut uki_cursor];
        let raw_blob = Blob {
            offset: blob_offset,
            size: u64::try_from(blob.len()).unwrap_or(0),
            reader: &mut blob_cursor,
        };

        // ACT
        build(
            &layout,
            &mut readers,
            &mut [raw_blob],
            blob_offset
                .saturating_add(u64::try_from(blob.len()).unwrap_or(0))
                .next_multiple_of(ALIGN_1_MIB_BYTES),
            &mut out,
            None,
        )
        .expect("raw::build must succeed with a blob");
        let img = out.into_inner();

        // ASSERT
        let blob_start = usize::try_from(blob_offset).unwrap_or(0);
        assert_eq!(
            &img[blob_start..blob_start + 4],
            &blob[0..4],
            "blob payload must appear at its byte offset"
        );
        let mut cursor = Cursor::new(&img);
        let gpt = io::read(&mut cursor).expect("valid GPT");
        let part = gpt.partition(1).expect("must have ESP partition");
        assert!(
            part.starting_lba.saturating_mul(SECTOR_SIZE)
                >= blob_offset.saturating_add(u64::try_from(blob.len()).unwrap_or(0)),
            "partition table must start at or after the blob end"
        );
    }

    #[test]
    fn raw_blob_overlapping_gpt_is_rejected() {
        // ARRANGE
        use std::io::Cursor;

        use esp::FileMeta;
        use esp::arch::Arch;
        use esp::layout::compute;

        let uki = vec![0xCC_u8; 1024];
        let uki_size = u64::try_from(uki.len()).unwrap_or(0);
        let mut uki_cursor = Cursor::new(uki);
        let mut blob_cursor = Cursor::new(vec![0xAB_u8; 512]);

        let files = &[FileMeta::new(Arch::X86_64.boot_path(), uki_size)];
        let layout = compute(files).expect("compute layout");

        let mut out = Cursor::new(Vec::new());
        let mut readers: Vec<&mut dyn Read> = vec![&mut uki_cursor];
        let raw_blob = Blob {
            offset: 512,
            size: 512,
            reader: &mut blob_cursor,
        };

        // ACT
        let result = build(
            &layout,
            &mut readers,
            &mut [raw_blob],
            ALIGN_1_MIB_BYTES,
            &mut out,
            None,
        );

        // ASSERT
        assert!(matches!(result, Err(MisoError::Gpt(_))));
    }
}
