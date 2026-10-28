//! Raw disk image writer with protective MBR + GPT + EFI System Partition.

use std::io::{Read, Write};

use esp::EFI_GUID;
use esp::image;
use esp::layout::Layout;
use parttable::gpt;
use parttable::gpt::layout::{ALIGN_1_MIB_SECTORS, PlacementRequest, Size, Slot, Start};
use parttable::gpt::plan as gpt_plan;
use parttable::gpt::plan::{ALIGN_1_MIB_BYTES, SECTOR_SIZE, smallest_disk};
use parttable::gpt::table::Table;

use crate::error::{MisoError, Result};

/// Fixed disk GUID baked into raw images so builds are reproducible.
///
/// `sha256("muak/raw/disk")[..16]` with the version 4 and RFC 4122 variant bits set, which renders as
/// `21B77290-CF7E-4661-929A-094DCB36C99D`. The array holds the on-disk GPT byte order, so the version nibble
/// lives in byte 7 and the variant bits in byte 8.
pub const IMAGE_DISK_GUID: [u8; 16] = [
    0x90, 0x72, 0xb7, 0x21, 0x7e, 0xcf, 0x61, 0x46, 0x92, 0x9a, 0x09, 0x4d, 0xcb, 0x36, 0xc9, 0x9d,
];

/// Fixed partition GUID baked into raw images so builds are reproducible.
///
/// `sha256("muak/raw/esp")[..16]` with the version 4 and RFC 4122 variant bits set, which renders as
/// `E2E39F6A-7932-4C3E-A0EC-B35FF16064CB`. This is the value exposed as the ESP `PARTUUID`, so it must be a
/// well formed GUID and must not be the all-ones or all-zeros sentinel.
pub const IMAGE_PARTITION_GUID: [u8; 16] = [
    0x6a, 0x9f, 0xe3, 0xe2, 0x32, 0x79, 0x3e, 0x4c, 0xa0, 0xec, 0xb3, 0x5f, 0xf1, 0x60, 0x64, 0xcb,
];

/// Byte extent of a raw boot blob.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    /// Byte offset on the boot device where the blob is written.
    pub offset: u64,
    /// Size of the blob payload in bytes.
    pub size: u64,
}

/// A raw boot blob written at a fixed byte offset before the partition table.
pub struct Blob<'a> {
    /// Byte extent of the blob payload.
    pub extent: Extent,
    /// Streaming reader for the blob payload.
    pub reader: &'a mut dyn Read,
}

/// Planned raw disk geometry, computed without writing anything.
pub struct Plan {
    disk_sectors: u64,
    esp_size: u64,
    partition_start_lba: u64,
    extents: Vec<Extent>,
}

impl Plan {
    /// Total raw disk image size in bytes, before any transport codec.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.disk_sectors.saturating_mul(SECTOR_SIZE)
    }
}

/// Computes the raw disk geometry for `layout` and `extents` without writing anything.
///
/// # Errors
///
/// Returns an error when a blob overlaps the GPT or another blob, or when the
/// ESP does not fit the requested disk geometry.
pub fn plan(layout: &Layout<'_>, extents: &[Extent]) -> Result<Plan> {
    let partition_start = partition_start(extents);
    validate_blobs(extents, partition_start)?;
    let partition_start_lba = partition_start.div_ceil(SECTOR_SIZE);
    let request = efi_request(partition_start_lba, layout.total_size);
    let disk_sectors = smallest_disk(
        ALIGN_1_MIB_SECTORS * 2,
        SECTOR_SIZE,
        ALIGN_1_MIB_SECTORS,
        IMAGE_DISK_GUID,
        &request,
    )?;
    let mut extents = extents.to_vec();
    extents.sort_by_key(|extent| extent.offset);

    Ok(Plan {
        disk_sectors,
        esp_size: layout.total_size,
        partition_start_lba,
        extents,
    })
}

/// Writes the raw disk image described by `plan`.
///
/// # Errors
///
/// Returns an error when `layout` or the blob extents no longer match `plan`,
/// GPT construction fails, ESP construction fails, or output writing fails.
pub fn write<'data, W: Write>(
    plan: &Plan,
    layout: &Layout<'data>,
    esp: &mut [&'data mut (dyn Read + 'data)],
    blobs: &mut [Blob<'data>],
    out: &mut W,
) -> Result<()> {
    if layout.total_size != plan.esp_size {
        return Err(MisoError::Gpt(format!(
            "raw plan ESP size {} does not match layout size {}",
            plan.esp_size, layout.total_size
        )));
    }

    blobs.sort_by_key(|blob| blob.extent.offset);
    let provided = blobs.iter().map(|blob| blob.extent);
    if !provided.eq(plan.extents.iter().copied()) {
        return Err(MisoError::Gpt(
            "raw plan blob extents do not match the provided blobs".to_owned(),
        ));
    }

    let request = efi_request(plan.partition_start_lba, plan.esp_size);
    let mut table = Table::create(plan.disk_sectors, SECTOR_SIZE, IMAGE_DISK_GUID)?;
    let placement = request.place(&mut table, SECTOR_SIZE)?;
    let partition_offset = placement
        .partition
        .starting_lba
        .checked_mul(SECTOR_SIZE)
        .ok_or(MisoError::Gpt("raw image arithmetic overflowed".to_owned()))?;

    gpt::io::write_primary(&table, plan.disk_sectors, out)?;
    let mut pos = table.primary_gpt_size();

    for blob in blobs.iter_mut() {
        write_zeros(out, blob.extent.offset.saturating_sub(pos))?;
        let written = std::io::copy(&mut blob.reader, out)?;
        pos = blob.extent.offset.saturating_add(written);
    }

    write_zeros(out, partition_offset.saturating_sub(pos))?;
    image::build(layout, esp, out).map_err(MisoError::Esp)?;
    pos = partition_offset.saturating_add(plan.esp_size);
    let backup_offset = table.backup_data_offset(plan.disk_sectors);
    write_zeros(out, backup_offset.saturating_sub(pos))?;
    gpt::io::write_backup(&table, plan.disk_sectors, out)?;

    Ok(())
}

fn partition_start(extents: &[Extent]) -> u64 {
    let end = extents
        .iter()
        .map(|extent| extent.offset.saturating_add(extent.size))
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

fn validate_blobs(extents: &[Extent], partition_start: u64) -> Result<()> {
    let gpt_end = gpt_plan::primary_gpt_size(SECTOR_SIZE);
    let mut prev_end = 0_u64;
    let mut sorted: Vec<&Extent> = extents.iter().collect();
    sorted.sort_by_key(|extent| extent.offset);

    for extent in sorted {
        if extent.offset < gpt_end {
            return Err(MisoError::Gpt(format!(
                "blob at offset {} overlaps the primary GPT region",
                extent.offset
            )));
        }
        if extent.offset.saturating_add(extent.size) > partition_start {
            return Err(MisoError::Gpt(format!(
                "blob at offset {} extends past the partition table start {partition_start}",
                extent.offset
            )));
        }
        if extent.offset < prev_end {
            return Err(MisoError::Gpt(format!(
                "blob at offset {} overlaps an earlier blob",
                extent.offset
            )));
        }
        prev_end = extent.offset.saturating_add(extent.size);
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
        let extents = [
            Extent {
                offset: 32 * 1024,
                size: 1024,
            },
            Extent {
                offset: 2 * ALIGN_1_MIB_BYTES + 4096,
                size: 512,
            },
        ];

        // ACT
        let start = partition_start(&extents);

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
        let extent = Extent {
            offset: blob_offset,
            size: u64::try_from(blob.len()).unwrap_or(0),
        };
        let planned = plan(&layout, &[extent]).expect("raw::plan must succeed");

        let mut out = Cursor::new(Vec::new());
        let mut readers: Vec<&mut dyn Read> = vec![&mut uki_cursor];
        let raw_blob = Blob {
            extent,
            reader: &mut blob_cursor,
        };

        // ACT
        write(&planned, &layout, &mut readers, &mut [raw_blob], &mut out)
            .expect("raw::write must succeed with a blob");
        let img = out.into_inner();

        // ASSERT
        assert_eq!(
            usize::try_from(planned.size()).unwrap_or(0),
            img.len(),
            "planned size must match the emitted image"
        );
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

        let files = &[FileMeta::new(Arch::X86_64.boot_path(), uki_size)];
        let layout = compute(files).expect("compute layout");
        let extent = Extent {
            offset: 512,
            size: 512,
        };

        // ACT
        let result = plan(&layout, &[extent]);

        // ASSERT
        assert!(matches!(result, Err(MisoError::Gpt(_))));
    }
}
