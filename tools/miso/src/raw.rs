//! Raw disk image writer with protective MBR + GPT + EFI System Partition.

use std::io::{Read, Write};

use esp::EFI_GUID;
use esp::image;
use esp::layout::Layout;
use parttable::gpt;
use parttable::gpt::layout::{ALIGN_1_MIB_SECTORS, PlacementRequest, Size, Slot, Start};
use parttable::gpt::plan;
use parttable::gpt::plan::{ALIGN_1_MIB_BYTES, SECTOR_SIZE, smallest_disk};
use parttable::gpt::table::Table;

use crate::error::{MisoError, Result};

/// Fixed disk GUID baked into raw images so builds are reproducible.
pub const IMAGE_DISK_GUID: [u8; 16] = [0xff; 16];

/// Fixed partition GUID baked into raw images so builds are reproducible.
pub const IMAGE_PARTITION_GUID: [u8; 16] = [0xaa; 16];

/// The default zstd compression level used when compression is requested.
pub const DEFAULT_ZSTD_LEVEL: i32 = 6;

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
/// The first partition starts at the smallest 1 MiB-aligned offset past the
/// last blob (at least 1 MiB), so blobs never overlap the GPT or the ESP.
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
    out: &mut W,
    compression_level: Option<i32>,
) -> Result<()> {
    let partition_start = partition_start(blobs);
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

/// Returns the smallest 1 MiB-aligned partition start that sits past every blob.
fn partition_start(blobs: &[Blob]) -> u64 {
    let end = blobs
        .iter()
        .map(|blob| blob.offset.saturating_add(blob.size))
        .max()
        .unwrap_or(0);

    end.next_multiple_of(ALIGN_1_MIB_BYTES)
        .max(ALIGN_1_MIB_BYTES)
}

fn efi_request(start_lba: u64, esp_size: u64) -> PlacementRequest {
    PlacementRequest::new(EFI_GUID, IMAGE_PARTITION_GUID, "EFI", Size::Bytes(esp_size))
        .slot(Slot::Exact(1))
        .start(Start::AtOrAfter(start_lba))
}

fn write<W: Write, B: FnOnce(&mut W) -> Result<()>>(
    out: &mut W,
    esp_size: u64,
    partition_start: u64,
    blobs: &mut [Blob],
    esp_builder: B,
) -> Result<()> {
    let start_lba = partition_start.div_ceil(SECTOR_SIZE);
    let request = efi_request(start_lba, esp_size);
    let disk_sectors = smallest_disk(
        ALIGN_1_MIB_SECTORS * 2,
        SECTOR_SIZE,
        ALIGN_1_MIB_SECTORS,
        IMAGE_DISK_GUID,
        &request,
    )?;

    let mut table = Table::create(disk_sectors, SECTOR_SIZE, IMAGE_DISK_GUID)?;
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
    let gpt_end = plan::primary_gpt_size(SECTOR_SIZE);
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
    use std::io::Cursor;

    use esp::FileMeta;
    use esp::arch::Arch;
    use esp::layout::compute;
    use parttable::gpt::io;

    use super::*;
    use crate::error::MisoError;

    #[test]
    fn partition_start_defaults_to_one_mib() {
        // ARRANGE / ACT
        let start = partition_start(&[]);

        // ASSERT
        assert_eq!(start, ALIGN_1_MIB_BYTES);
    }

    #[test]
    fn partition_start_aligns_past_the_last_blob() {
        // ARRANGE
        let mut first_reader = Cursor::new(Vec::new());
        let mut last_reader = Cursor::new(Vec::new());
        let blobs = [
            Blob {
                offset: 32 * 1024,
                size: 1024,
                reader: &mut first_reader,
            },
            Blob {
                offset: 2 * ALIGN_1_MIB_BYTES + 4096,
                size: 512,
                reader: &mut last_reader,
            },
        ];

        // ACT
        let start = partition_start(&blobs);

        // ASSERT
        let last_end = 2 * ALIGN_1_MIB_BYTES + 4096 + 512;
        assert_eq!(start, last_end.next_multiple_of(ALIGN_1_MIB_BYTES));
    }

    #[test]
    fn raw_blob_written_at_offset_and_partition_shifted() {
        // ARRANGE
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
        build(&layout, &mut readers, &mut [raw_blob], &mut out, None)
            .expect("raw::build must succeed with a blob");
        let img = out.into_inner();

        // ASSERT
        let blob_start = usize::try_from(blob_offset).unwrap_or(0);
        let img_bytes = img.as_slice();
        let blob_bytes = blob.as_slice();
        assert_eq!(
            img_bytes.get(blob_start..blob_start + 4),
            blob_bytes.get(0..4),
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
        let result = build(&layout, &mut readers, &mut [raw_blob], &mut out, None);

        // ASSERT
        assert!(matches!(result, Err(MisoError::Gpt(_))));
    }
}
