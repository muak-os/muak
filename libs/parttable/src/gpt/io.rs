//! GPT streaming I/O.

use std::io::{Read, Write};

use super::header::GptHeader;
use super::partition::Partition;
use super::table::{ENTRIES_COUNT, ENTRY_SIZE, Table};
use crate::error::{ParttableError, Result};
use crate::mbr::protective_bytes;

/// Total size in bytes of a 128-entry partition entries array.
const ENTRIES_BYTES: usize = 16_384;

/// Reads a GPT table from a forward-only reader.
///
/// Only the primary GPT copy is read; the backup copy is not consulted.
/// The sector size is probed (512 then 4096 bytes) like the GPT spec allows.
///
/// # Errors
///
/// Returns an error when the stream is truncated, the header or entries are
/// corrupt, or the entries geometry is unsupported.
pub fn read<R: Read>(reader: &mut R) -> Result<Table> {
    let mut buffer = [0_u8; 512];
    reader.read_exact(&mut buffer)?;
    let mut position = 512_u64;

    let mut sector_size = 512_u64;
    reader.read_exact(&mut buffer)?;
    position = position.saturating_add(512);
    if !buffer.starts_with(b"EFI PART") {
        skip(reader, 4096_u64.saturating_sub(position))?;
        position = 4096;
        reader.read_exact(&mut buffer)?;
        position = position.saturating_add(512);
        sector_size = 4096;
    }

    let header = GptHeader::parse(&buffer)?;
    validate_geometry(&header)?;

    let entries_offset = header
        .entries_lba
        .checked_mul(sector_size)
        .ok_or_else(|| ParttableError::Gpt("partition entries LBA overflowed".to_owned()))?;
    if entries_offset > position {
        skip(reader, entries_offset.saturating_sub(position))?;
    }

    let mut entries = [0_u8; ENTRIES_BYTES];
    reader.read_exact(&mut entries)?;
    if header.entries_crc != crc32fast::hash(&entries) {
        return Err(ParttableError::Gpt(
            "partition entries CRC mismatch".to_owned(),
        ));
    }

    decode_table(header, sector_size, &entries)
}

/// Writes the protective MBR, GPT header, and partition entries sequentially.
///
/// # Errors
///
/// Returns an error when writing the data fails.
pub fn write_primary<W: Write>(table: &Table, sector_count: u64, writer: &mut W) -> Result<()> {
    let disk_size = sector_count
        .checked_mul(table.sector_size())
        .ok_or_else(|| ParttableError::Gpt("disk size overflowed".to_owned()))?;
    let protective_mbr = protective_bytes(disk_size, table.sector_size());
    writer.write_all(&protective_mbr)?;

    write_region(table, sector_count, false, writer)
}

/// Writes the backup GPT (partition entries + header) sequentially.
///
/// # Errors
///
/// Returns an error when writing the data fails.
pub fn write_backup<W: Write>(table: &Table, sector_count: u64, writer: &mut W) -> Result<()> {
    write_region(table, sector_count, true, writer)
}

fn write_region<W: Write>(
    table: &Table,
    sector_count: u64,
    backup: bool,
    writer: &mut W,
) -> Result<()> {
    let entries = partition_entries_bytes(table);
    let entries_crc = crc32fast::hash(&entries);
    let header = table
        .to_header()
        .encode(backup, sector_count, table.sector_size(), entries_crc);
    if backup {
        writer.write_all(&entries)?;
    }
    writer.write_all(&header)?;
    if !backup {
        writer.write_all(&entries)?;
    }

    Ok(())
}

fn partition_entries_bytes(table: &Table) -> [u8; ENTRIES_BYTES] {
    let mut buffer = [0_u8; ENTRIES_BYTES];
    for (number, partition) in table.partitions() {
        let encoded = partition.encode();
        let offset = usize::try_from(number.saturating_sub(1))
            .unwrap_or(0)
            .saturating_mul(usize::try_from(ENTRY_SIZE).unwrap_or(0));
        let end = offset.saturating_add(usize::try_from(ENTRY_SIZE).unwrap_or(0));
        if let Some(dst) = buffer.get_mut(offset..end) {
            dst.copy_from_slice(&encoded);
        }
    }

    buffer
}

fn decode_table(
    header: GptHeader,
    sector_size: u64,
    entries: &[u8; ENTRIES_BYTES],
) -> Result<Table> {
    let mut parsed = Vec::with_capacity(ENTRIES_COUNT);
    for chunk in entries.chunks_exact(usize::try_from(ENTRY_SIZE).unwrap_or(0)) {
        let mut entry = [0_u8; 128];
        entry.copy_from_slice(chunk);
        parsed.push(Partition::decode(&entry));
    }

    Table::from_parts(
        header.first_usable_lba,
        header.last_usable_lba,
        header.disk_guid,
        sector_size,
        parsed,
    )
}

fn validate_geometry(header: &GptHeader) -> Result<()> {
    if header.entries_count != u32::try_from(ENTRIES_COUNT).unwrap_or(0) {
        return Err(ParttableError::Gpt(
            "unsupported partition entries count".to_owned(),
        ));
    }
    if header.entries_size != ENTRY_SIZE {
        return Err(ParttableError::Gpt(
            "unsupported partition entry size".to_owned(),
        ));
    }

    Ok(())
}

fn skip<R: Read>(reader: &mut R, remaining: u64) -> Result<()> {
    let copied = std::io::copy(&mut reader.take(remaining), &mut std::io::sink())?;
    if copied < remaining {
        return Err(ParttableError::Gpt(
            "stream ended before GPT entries".to_owned(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use esp::EFI_GUID;

    use super::*;
    use crate::gpt::partition::{LINUX_FS_GUID, Partition};

    fn sample_table() -> Table {
        let sector_count = 8 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(
            1,
            Partition {
                type_guid: EFI_GUID,
                unique_guid: [0xAB; 16],
                starting_lba: 2048,
                ending_lba: 4095,
                attributes: 0,
                name: "EFI".to_owned(),
            },
        );
        table
    }

    #[test]
    fn write_then_read_round_trips_primary() {
        // ARRANGE
        let sector_count = 8 * 2048;
        let table = sample_table();

        // ACT
        let mut buf = Vec::new();
        write_primary(&table, sector_count, &mut buf).expect("primary write must succeed");
        let mut reader: &[u8] = &buf;
        let reread = read(&mut reader).expect("table must be readable");

        // ASSERT
        let partition = reread.partition(1).expect("partition must exist");
        assert_eq!(partition.type_guid, EFI_GUID);
        assert_eq!(partition.starting_lba, 2048);
        assert_eq!(partition.ending_lba, 4095);
        assert_eq!(partition.name, "EFI");
    }

    #[test]
    fn read_accepts_full_disk_image_with_backup_at_end() {
        // ARRANGE
        let sector_count = 8 * 2048;
        let table = sample_table();
        let mut disk = Vec::new();
        write_primary(&table, sector_count, &mut disk).expect("primary write must succeed");
        let backup_offset = table.backup_data_offset(sector_count);
        disk.resize(usize::try_from(backup_offset).unwrap_or(0), 0);
        write_backup(&table, sector_count, &mut disk).expect("backup write must succeed");

        // ACT
        let mut reader: &[u8] = &disk;
        let reread = read(&mut reader).expect("table must be readable");

        // ASSERT
        assert_eq!(reread.partition(1).expect("partition").name, "EFI");
        assert_eq!(
            disk.len(),
            usize::try_from(sector_count.saturating_mul(512)).unwrap_or(0)
        );
    }

    #[test]
    fn primary_region_matches_golden_fixture() {
        // ARRANGE
        let sector_count = 8 * 2048;
        let table = sample_table();
        let mut buf = Vec::new();

        // ACT
        write_primary(&table, sector_count, &mut buf).expect("primary write must succeed");

        // ASSERT
        assert_eq!(buf, include_bytes!("../../tests/fixtures/primary.bin"));
    }

    #[test]
    fn backup_region_matches_golden_fixture() {
        // ARRANGE
        let sector_count = 8 * 2048;
        let table = sample_table();
        let mut buf = Vec::new();

        // ACT
        write_backup(&table, sector_count, &mut buf).expect("backup write must succeed");

        // ASSERT
        assert_eq!(buf, include_bytes!("../../tests/fixtures/backup.bin"));
    }

    #[test]
    fn read_rejects_corrupt_entries_crc() {
        // ARRANGE
        let sector_count = 8 * 2048;
        let table = sample_table();
        let mut buf = Vec::new();
        write_primary(&table, sector_count, &mut buf).expect("primary write must succeed");
        let entries_start = 1024_usize;
        if let Some(byte) = buf.get_mut(entries_start) {
            *byte = byte.wrapping_add(1);
        }

        // ACT
        let mut reader: &[u8] = &buf;
        let result = read(&mut reader);

        // ASSERT
        assert!(matches!(
            result,
            Err(ParttableError::Gpt(message)) if message == "partition entries CRC mismatch"
        ));
    }

    #[test]
    fn read_rejects_non_gpt_stream() {
        // ARRANGE
        let stream = vec![0_u8; 32 * 1024];

        // ACT
        let mut reader: &[u8] = stream.as_slice();
        let result = read(&mut reader);

        // ASSERT
        assert!(matches!(result, Err(ParttableError::Gpt(_))));
    }

    #[test]
    fn read_rejects_entries_lba_overflow() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut header = Table::create(sector_count, 512, [0xCD; 16])
            .expect("table must be created")
            .to_header();
        header.entries_lba = u64::MAX;
        let header_sector = header.encode(false, sector_count, 512, 0);
        assert_eq!(
            header_sector.get(72..80),
            Some(&u64::MAX.to_le_bytes()[..]),
            "entries_lba must be encoded"
        );

        let mut disk = Vec::new();
        disk.extend_from_slice(&[0_u8; 512]);
        disk.extend_from_slice(&header_sector);

        // ACT
        let mut reader: &[u8] = disk.as_slice();
        let result = read(&mut reader);

        // ASSERT
        let message = match result {
            Err(ParttableError::Gpt(message)) if message == "partition entries LBA overflowed" => {
                message
            }
            other => panic!("unexpected result: {other:?}"),
        };
        assert_eq!(message, "partition entries LBA overflowed");
    }

    #[test]
    fn write_primary_rejects_disk_size_overflow() {
        // ARRANGE
        let table = sample_table();
        let mut buf = Vec::new();

        // ACT
        let result = write_primary(&table, u64::MAX, &mut buf);

        // ASSERT
        assert!(matches!(
            result,
            Err(ParttableError::Gpt(message)) if message == "disk size overflowed"
        ));
    }

    #[test]
    fn read_uses_last_usable_lba_from_header() {
        // ARRANGE
        let sector_count = 8 * 2048;
        let table = sample_table();
        let mut buf = Vec::new();
        write_primary(&table, sector_count, &mut buf).expect("primary write must succeed");

        // ACT
        let mut reader: &[u8] = &buf;
        let reread = read(&mut reader).expect("table must be readable");

        // ASSERT
        assert_eq!(reread.last_usable_lba(), table.last_usable_lba());
    }

    #[test]
    fn read_probes_4096_byte_sectors() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let entries = [0_u8; ENTRIES_BYTES];
        let entries_crc = crc32fast::hash(&entries);
        let header = Table::create(sector_count, 4096, [0xCD; 16])
            .expect("table must be created")
            .to_header()
            .encode(false, sector_count, 4096, entries_crc);

        let mut disk = Vec::new();
        disk.extend_from_slice(&[0_u8; 4096]);
        disk.extend_from_slice(&header);
        disk.resize(8192_usize.saturating_add(ENTRIES_BYTES), 0);
        if let Some(dst) = disk.get_mut(8192_usize..8192_usize.saturating_add(ENTRIES_BYTES)) {
            dst.copy_from_slice(&entries);
        }

        // ACT
        let mut reader: &[u8] = &disk;
        let table = read(&mut reader).expect("4096-byte sector GPT must be readable");

        // ASSERT
        assert_eq!(table.sector_size(), 4096);
        assert!(!table.has_used_partitions());
    }

    #[test]
    fn read_rejects_truncated_stream_while_skipping() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let entries = [0_u8; ENTRIES_BYTES];
        let entries_crc = crc32fast::hash(&entries);
        let header = Table::create(sector_count, 4096, [0xCD; 16])
            .expect("table must be created")
            .to_header()
            .encode(false, sector_count, 4096, entries_crc);

        let mut disk = Vec::new();
        disk.extend_from_slice(&[0_u8; 4096]);
        disk.extend_from_slice(&header);

        // ACT
        let mut reader: &[u8] = disk.as_slice();
        let result = read(&mut reader);

        // ASSERT
        assert!(matches!(
            result,
            Err(ParttableError::Gpt(message)) if message == "stream ended before GPT entries"
        ));
    }

    #[test]
    fn validate_geometry_rejects_foreign_entries_geometry() {
        // ARRANGE
        let header = Table::create(16 * 2048, 512, [0xCD; 16])
            .expect("table must be created")
            .to_header();

        // ACT
        let wrong_count = {
            let mut other = header;
            other.entries_count = 64;
            validate_geometry(&other)
        };
        let wrong_size = {
            let mut other = header;
            other.entries_size = 256;
            validate_geometry(&other)
        };

        // ASSERT
        assert!(matches!(
            wrong_count,
            Err(ParttableError::Gpt(message)) if message == "unsupported partition entries count"
        ));
        assert!(matches!(
            wrong_size,
            Err(ParttableError::Gpt(message)) if message == "unsupported partition entry size"
        ));
    }

    #[test]
    fn entries_region_of_a_disk_with_unused_slots_is_consistent() {
        // ARRANGE
        let sector_count = 8 * 2048;
        let mut table = sample_table();
        table.set_partition(
            2,
            Partition {
                type_guid: LINUX_FS_GUID,
                unique_guid: [0xBC; 16],
                starting_lba: 4096,
                ending_lba: 8191,
                attributes: 0,
                name: "STATE".to_owned(),
            },
        );
        let mut buf = Vec::new();

        // ACT
        write_primary(&table, sector_count, &mut buf).expect("primary write must succeed");
        let mut reader: &[u8] = &buf;
        let reread = read(&mut reader).expect("table must be readable");

        // ASSERT
        let used = reread.used_partitions();
        assert_eq!(used.len(), 2);
        assert_eq!(used.first().map(|entry| entry.0), Some(1));
        assert_eq!(used.get(1).map(|entry| entry.0), Some(2));
    }
}
