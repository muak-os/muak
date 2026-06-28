//! GPT byte-level serialization helpers.

use std::io::Write;

use gptman::{GPT, GPTPartitionEntry};

use crate::error::Result;

/// Size of the GPT header in bytes.
pub(crate) const GPT_HEADER_SIZE: usize = 92;

// GPT header field offsets within the 92-byte header
const HDR_SIGNATURE: core::ops::Range<usize> = 0..8;
const HDR_REVISION: core::ops::Range<usize> = 8..12;
const HDR_SIZE: core::ops::Range<usize> = 12..16;
const HDR_CRC: core::ops::Range<usize> = 16..20;
const HDR_CURRENT_LBA: core::ops::Range<usize> = 24..32;
const HDR_BACKUP_LBA: core::ops::Range<usize> = 32..40;
const HDR_FIRST_USABLE: core::ops::Range<usize> = 40..48;
const HDR_LAST_USABLE: core::ops::Range<usize> = 48..56;
const HDR_DISK_GUID: core::ops::Range<usize> = 56..72;
const HDR_ENTRIES_LBA: core::ops::Range<usize> = 72..80;
const HDR_ENTRIES_COUNT: core::ops::Range<usize> = 80..84;
const HDR_ENTRY_SIZE: core::ops::Range<usize> = 84..88;
const HDR_ENTRIES_CRC: core::ops::Range<usize> = 88..92;

// Partition entry field offsets (128 bytes per entry)
const ENT_TYPE_GUID: core::ops::Range<usize> = 0..16;
const ENT_UNIQUE_GUID: core::ops::Range<usize> = 16..32;
const ENT_STARTING_LBA: core::ops::Range<usize> = 32..40;
const ENT_ENDING_LBA: core::ops::Range<usize> = 40..48;
const ENT_ATTRIBUTES: core::ops::Range<usize> = 48..56;
const ENT_NAME_START: usize = 56;
const ENT_NAME_MAX_BYTES: usize = 72;

/// Builds a 92-byte GPT header and returns the CRC32 of the partition entries.
#[must_use]
pub(crate) fn gpt_header_bytes(
    gpt: &GPT,
    backup: bool,
    sector_count: u64,
) -> ([u8; GPT_HEADER_SIZE], u32) {
    let mut hdr = [0_u8; GPT_HEADER_SIZE];
    hdr[HDR_SIGNATURE].copy_from_slice(b"EFI PART");
    hdr[HDR_REVISION].copy_from_slice(&0x0001_0000_u32.to_le_bytes());
    hdr[HDR_SIZE].copy_from_slice(&u32::try_from(GPT_HEADER_SIZE).unwrap_or(92).to_le_bytes());

    let current_lba = if backup {
        sector_count.saturating_sub(1)
    } else {
        1
    };
    let backup_lba = if backup {
        1
    } else {
        sector_count.saturating_sub(1)
    };
    hdr[HDR_CURRENT_LBA].copy_from_slice(&current_lba.to_le_bytes());
    hdr[HDR_BACKUP_LBA].copy_from_slice(&backup_lba.to_le_bytes());
    hdr[HDR_FIRST_USABLE].copy_from_slice(&gpt.header.first_usable_lba.to_le_bytes());
    hdr[HDR_LAST_USABLE].copy_from_slice(&gpt.header.last_usable_lba.to_le_bytes());
    hdr[HDR_DISK_GUID].copy_from_slice(&gpt.header.disk_guid);

    let entries_lba = if backup {
        sector_count.saturating_sub(1).saturating_sub(
            u64::from(gpt.header.number_of_partition_entries)
                .saturating_mul(u64::from(gpt.header.size_of_partition_entry))
                .div_ceil(gpt.sector_size),
        )
    } else {
        gpt.header.partition_entry_lba
    };
    hdr[HDR_ENTRIES_LBA].copy_from_slice(&entries_lba.to_le_bytes());
    hdr[HDR_ENTRIES_COUNT].copy_from_slice(&gpt.header.number_of_partition_entries.to_le_bytes());
    hdr[HDR_ENTRY_SIZE].copy_from_slice(&gpt.header.size_of_partition_entry.to_le_bytes());

    let entries = partition_entries_bytes(gpt);

    (hdr, crc32fast::hash(&entries))
}

/// Serializes all partition entries into a byte buffer.
#[must_use]
pub(crate) fn partition_entries_bytes(gpt: &GPT) -> Vec<u8> {
    let count = usize::try_from(gpt.header.number_of_partition_entries).unwrap_or(0);
    let entry_size = usize::try_from(gpt.header.size_of_partition_entry).unwrap_or(0);
    let mut buf = vec![0_u8; count.saturating_mul(entry_size)];

    for (i, entry) in gpt.iter() {
        let offset = i
            .saturating_sub(1)
            .saturating_mul(u32::try_from(entry_size).unwrap_or(0));
        let offset = usize::try_from(offset).unwrap_or(0);
        let Some(dst) = buf.get_mut(offset..offset.saturating_add(128)) else {
            continue;
        };
        let entry_bytes = partition_entry_to_bytes(entry);
        dst.copy_from_slice(&entry_bytes);
    }

    buf
}

/// Writes a padded GPT header (92 bytes + 420 bytes padding = 512 bytes total).
pub(crate) fn write_gpt_header<W: Write>(
    header: &[u8; GPT_HEADER_SIZE],
    writer: &mut W,
) -> Result<()> {
    writer.write_all(header)?;
    let pad = [0_u8; 512 - GPT_HEADER_SIZE];
    writer.write_all(&pad)?;

    Ok(())
}

/// Computes and embeds the CRC32 checksums into a GPT header.
#[must_use]
pub(crate) fn finalize_gpt_header(
    mut header: [u8; GPT_HEADER_SIZE],
    entries_crc: u32,
) -> [u8; GPT_HEADER_SIZE] {
    header[HDR_ENTRIES_CRC].copy_from_slice(&entries_crc.to_le_bytes());
    let header_crc = crc32fast::hash(&header);
    header[HDR_CRC].copy_from_slice(&header_crc.to_le_bytes());

    header
}

/// Serializes a single partition entry into 128 bytes.
#[must_use]
fn partition_entry_to_bytes(entry: &GPTPartitionEntry) -> [u8; 128] {
    let mut bytes = [0_u8; 128];
    bytes[ENT_TYPE_GUID].copy_from_slice(&entry.partition_type_guid);
    bytes[ENT_UNIQUE_GUID].copy_from_slice(&entry.unique_partition_guid);
    bytes[ENT_STARTING_LBA].copy_from_slice(&entry.starting_lba.to_le_bytes());
    bytes[ENT_ENDING_LBA].copy_from_slice(&entry.ending_lba.to_le_bytes());
    bytes[ENT_ATTRIBUTES].copy_from_slice(&entry.attribute_bits.to_le_bytes());
    let name_bytes: Vec<u8> = entry
        .partition_name
        .as_str()
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let name_len = name_bytes.len().min(ENT_NAME_MAX_BYTES);
    let name_end = ENT_NAME_START.saturating_add(name_len);
    if let Some(dst) = bytes.get_mut(ENT_NAME_START..name_end) {
        dst.copy_from_slice(name_bytes.get(..name_len).unwrap_or(&[]));
    }

    bytes
}
