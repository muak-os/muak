//! GPT table wrapper with a stable workspace-local API.

use std::io::{Cursor, Read, Seek, Write};

use gptman::{GPT, GPTPartitionEntry, PartitionName};

use super::types::{Partition, Placement, PlacementRequest, Size, Slot, Start};
use crate::error::{ParttableError, Result};

/// A GPT table wrapper with a stable workspace-local API.
#[derive(Debug)]
pub struct Table {
    pub(crate) inner: GPT,
}

impl Table {
    /// Creates a new GPT from known disk geometry.
    ///
    /// # Errors
    ///
    /// Returns an error when GPT initialization or arithmetic overflows.
    pub fn create(sector_count: u64, sector_size: u64, disk_guid: [u8; 16]) -> Result<Self> {
        let size = usize::try_from(sector_count.saturating_mul(sector_size))
            .map_err(|_err| ParttableError::Gpt("disk size overflow".to_owned()))?;
        let inner = GPT::new_from(&mut Cursor::new(vec![0; size]), sector_size, disk_guid)
            .map_err(|e| ParttableError::Gpt(e.to_string()))?;

        Ok(Self { inner })
    }

    /// Reads an existing GPT from `reader`.
    ///
    /// # Errors
    ///
    /// Returns an error when the GPT cannot be decoded from `reader`.
    pub fn read<R: Read + Seek>(reader: &mut R) -> Result<Self> {
        let inner = GPT::find_from(reader).map_err(|e| ParttableError::Gpt(e.to_string()))?;

        Ok(Self { inner })
    }

    /// Returns the first usable LBA from the GPT header.
    #[must_use]
    pub fn first_usable_lba(&self) -> u64 {
        self.inner.header.first_usable_lba
    }

    /// Returns the last usable LBA from the GPT header.
    #[must_use]
    pub fn last_usable_lba(&self) -> u64 {
        self.inner.header.last_usable_lba
    }

    /// Returns all used partitions as `(number, partition)` pairs.
    #[must_use]
    pub fn used_partitions(&self) -> Vec<(u32, Partition)> {
        self.inner
            .iter()
            .filter(|&(_, entry)| entry.is_used())
            .map(|(number, entry)| (number, Partition::from(entry)))
            .collect()
    }

    /// Returns `true` when the table contains any used partition.
    #[must_use]
    pub fn has_used_partitions(&self) -> bool {
        self.inner.iter().any(|(_, entry)| entry.is_used())
    }

    /// Returns the used partition at `number`, if present.
    #[must_use]
    pub fn partition(&self, number: u32) -> Option<Partition> {
        self.inner
            .iter()
            .find(|&(entry_number, entry)| entry_number == number && entry.is_used())
            .map(|(_, entry)| Partition::from(entry))
    }

    /// Returns `true` when `number` refers to a used partition.
    #[must_use]
    pub fn is_partition_used(&self, number: u32) -> bool {
        self.partition(number).is_some()
    }

    /// Returns the highest used partition number, if any.
    #[must_use]
    pub fn highest_used_partition_number(&self) -> Option<u32> {
        self.inner
            .iter()
            .filter(|&(_, entry)| entry.is_used())
            .map(|(number, _)| number)
            .max()
    }

    /// Returns the last used ending LBA, if any.
    #[must_use]
    pub fn last_used_ending_lba(&self) -> Option<u64> {
        self.inner
            .iter()
            .filter(|&(_, entry)| entry.is_used())
            .map(|(_, entry)| entry.ending_lba)
            .max()
    }

    /// Returns the next free partition number, if any.
    #[must_use]
    pub fn next_free_slot(&self) -> Option<u32> {
        let max_slots = self.inner.iter().map(|(number, _)| number).max()?;
        (1..=max_slots).find(|&number| !self.is_partition_used(number))
    }

    /// Sets `number` to `partition`.
    pub fn set_partition(&mut self, number: u32, partition: Partition) {
        self.inner[number] = partition.into();
    }

    /// Places one partition using checked alignment and range rules.
    ///
    /// # Errors
    ///
    /// Returns an error when slot selection, sizing, alignment, or range validation fails.
    pub fn place_partition(
        &mut self,
        request: PlacementRequest,
        sector_size: u64,
    ) -> Result<Placement> {
        let number = match request.slot {
            Slot::Auto => self.next_free_slot().ok_or_else(|| {
                ParttableError::InvalidPlacement("no free GPT partition slots".to_owned())
            })?,
            Slot::Exact(number) => self.resolve_exact_slot(number)?,
        };

        let anchor = self.resolve_start_anchor(request.start)?;
        let start = align_up_lba(anchor, request.alignment_lba);
        let end = self.resolve_end_lba(start, request.size, sector_size)?;
        self.validate_partition_range(number, start, end)?;

        let partition = Partition {
            type_guid: request.type_guid,
            unique_guid: request.unique_guid,
            starting_lba: start,
            ending_lba: end,
            attributes: request.attributes,
            name: request.name,
        };
        self.set_partition(number, partition.clone());

        Ok(Placement { number, partition })
    }

    /// Removes the partition at `number`.
    ///
    /// # Errors
    ///
    /// Returns an error when `number` cannot be removed from the underlying GPT.
    pub fn remove_partition(&mut self, number: u32) -> Result<()> {
        match self.inner.remove(number) {
            Ok(()) => Ok(()),
            Err(err) => Err(ParttableError::Gpt(err.to_string())),
        }
    }

    /// Writes the protective MBR, GPT header, and partition entries sequentially.
    ///
    /// # Errors
    ///
    /// Returns an error when writing the data fails.
    pub fn write_primary_to<W: Write>(&self, sector_count: u64, writer: &mut W) -> Result<()> {
        use crate::mbr::io;

        let mbr = io::protective_mbr_bytes(
            sector_count.saturating_mul(self.inner.sector_size),
            self.inner.sector_size,
        );
        writer.write_all(&mbr)?;

        let (header, entries_crc) = gpt_header_bytes(&self.inner, false, sector_count);
        let entries = partition_entries_bytes(&self.inner);

        let mut header_with_crc = header;
        header_with_crc[88..92].copy_from_slice(&entries_crc.to_le_bytes());
        let header_crc = crc32fast::hash(&header_with_crc);
        header_with_crc[16..20].copy_from_slice(&header_crc.to_le_bytes());

        write_gpt_header(&header_with_crc, writer)?;
        writer.write_all(&entries)?;

        Ok(())
    }

    /// Writes the backup GPT (partition entries + header) at the end of the disk.
    ///
    /// # Errors
    ///
    /// Returns an error when writing the data fails.
    pub fn write_backup_to<W: Write>(&self, sector_count: u64, writer: &mut W) -> Result<()> {
        let entries = partition_entries_bytes(&self.inner);
        let (header, entries_crc) = gpt_header_bytes(&self.inner, true, sector_count);

        let mut header_with_crc = header;
        header_with_crc[88..92].copy_from_slice(&entries_crc.to_le_bytes());
        let header_crc = crc32fast::hash(&header_with_crc);
        header_with_crc[16..20].copy_from_slice(&header_crc.to_le_bytes());

        writer.write_all(&entries)?;
        write_gpt_header(&header_with_crc, writer)?;

        Ok(())
    }

    fn resolve_start_anchor(&self, start: Start) -> Result<u64> {
        match start {
            Start::FirstUsable => Ok(self.first_usable_lba()),
            Start::AfterLastUsed => self.last_used_ending_lba().map_or_else(
                || Ok(self.first_usable_lba()),
                Self::checked_lba_after_last_used,
            ),
            Start::AtOrAfter(lba) => Ok(lba),
            Start::AfterPartition(number) => self
                .partition(number)
                .map(|partition| Self::checked_lba_after_partition(partition.ending_lba, number))
                .transpose()?
                .ok_or_else(|| {
                    ParttableError::InvalidPlacement(format!(
                        "cannot place after missing partition {number}"
                    ))
                }),
        }
    }

    fn resolve_exact_slot(&self, number: u32) -> Result<u32> {
        if !self.is_partition_used(number) {
            return Ok(number);
        }

        Err(ParttableError::InvalidPlacement(format!(
            "partition slot {number} is already in use"
        )))
    }

    fn resolve_end_lba(&self, start: u64, size: Size, sector_size: u64) -> Result<u64> {
        match size {
            Size::Bytes(bytes) => {
                let lbas = Self::nonzero_lbas(bytes.div_ceil(sector_size))?;
                Self::checked_end_lba(start, lbas)
            }
            Size::Lbas(lbas) => {
                let lbas = Self::nonzero_lbas(lbas)?;
                Self::checked_end_lba(start, lbas)
            }
            Size::FillToLastUsable => Ok(self.last_usable_lba()),
        }
    }

    fn checked_lba_after_last_used(ending_lba: u64) -> Result<u64> {
        ending_lba.checked_add(1).ok_or_else(|| {
            ParttableError::InvalidPlacement(
                "partition start LBA overflowed after last used partition".to_owned(),
            )
        })
    }

    fn checked_lba_after_partition(ending_lba: u64, number: u32) -> Result<u64> {
        ending_lba.checked_add(1).ok_or_else(|| {
            ParttableError::InvalidPlacement(format!(
                "partition start LBA overflowed after partition {number}"
            ))
        })
    }

    fn checked_end_lba(start: u64, lbas: u64) -> Result<u64> {
        start.checked_add(lbas.saturating_sub(1)).ok_or_else(|| {
            ParttableError::InvalidPlacement("partition end LBA overflowed".to_owned())
        })
    }

    fn nonzero_lbas(lbas: u64) -> Result<u64> {
        if lbas != 0 {
            return Ok(lbas);
        }

        Err(ParttableError::InvalidPlacement(
            "partition size must be greater than zero".to_owned(),
        ))
    }

    fn validate_partition_range(&self, number: u32, start: u64, end: u64) -> Result<()> {
        if start > end {
            return Err(ParttableError::InvalidPlacement(format!(
                "partition {number} start LBA {start} is after end LBA {end}"
            )));
        }
        if start < self.first_usable_lba() {
            return Err(ParttableError::InvalidPlacement(format!(
                "partition {number} starts before first usable LBA {}",
                self.first_usable_lba()
            )));
        }
        if end > self.last_usable_lba() {
            return Err(ParttableError::InvalidPlacement(format!(
                "partition {number} ends after last usable LBA {}",
                self.last_usable_lba()
            )));
        }

        if let Some(existing_number) =
            self.used_partitions()
                .into_iter()
                .find_map(|(existing_number, existing)| {
                    let overlaps_current_slot = existing_number == number;
                    let overlaps_range =
                        start <= existing.ending_lba && end >= existing.starting_lba;
                    (!overlaps_current_slot && overlaps_range).then_some(existing_number)
                })
        {
            return Err(ParttableError::InvalidPlacement(format!(
                "partition {number} overlaps partition {existing_number}"
            )));
        }

        Ok(())
    }
}

impl From<&GPTPartitionEntry> for Partition {
    fn from(entry: &GPTPartitionEntry) -> Self {
        Self {
            type_guid: entry.partition_type_guid,
            unique_guid: entry.unique_partition_guid,
            starting_lba: entry.starting_lba,
            ending_lba: entry.ending_lba,
            attributes: entry.attribute_bits,
            name: entry.partition_name.to_string(),
        }
    }
}

impl From<Partition> for GPTPartitionEntry {
    fn from(partition: Partition) -> Self {
        Self {
            partition_type_guid: partition.type_guid,
            unique_partition_guid: partition.unique_guid,
            starting_lba: partition.starting_lba,
            ending_lba: partition.ending_lba,
            attribute_bits: partition.attributes,
            partition_name: PartitionName::from(partition.name.as_str()),
        }
    }
}

/// Rounds `lba` up to the nearest multiple of `align`.
#[must_use]
pub fn align_up_lba(lba: u64, align: u64) -> u64 {
    lba.next_multiple_of(align)
}

fn gpt_header_bytes(gpt: &GPT, backup: bool, sector_count: u64) -> ([u8; 92], u32) {
    let mut hdr = [0_u8; 92];
    hdr[0..8].copy_from_slice(b"EFI PART");
    hdr[8..12].copy_from_slice(&0x0001_0000_u32.to_le_bytes());
    hdr[12..16].copy_from_slice(&92_u32.to_le_bytes());
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
    hdr[24..32].copy_from_slice(&current_lba.to_le_bytes());
    hdr[32..40].copy_from_slice(&backup_lba.to_le_bytes());
    hdr[40..48].copy_from_slice(&gpt.header.first_usable_lba.to_le_bytes());
    hdr[48..56].copy_from_slice(&gpt.header.last_usable_lba.to_le_bytes());
    hdr[56..72].copy_from_slice(&gpt.header.disk_guid);
    let entries_lba = if backup {
        sector_count.saturating_sub(1).saturating_sub(
            u64::from(gpt.header.number_of_partition_entries)
                .saturating_mul(u64::from(gpt.header.size_of_partition_entry))
                .div_ceil(gpt.sector_size),
        )
    } else {
        gpt.header.partition_entry_lba
    };
    hdr[72..80].copy_from_slice(&entries_lba.to_le_bytes());
    hdr[80..84].copy_from_slice(&gpt.header.number_of_partition_entries.to_le_bytes());
    hdr[84..88].copy_from_slice(&gpt.header.size_of_partition_entry.to_le_bytes());

    let entries = partition_entries_bytes(gpt);
    (hdr, crc32fast::hash(&entries))
}

fn partition_entries_bytes(gpt: &GPT) -> Vec<u8> {
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

fn partition_entry_to_bytes(entry: &GPTPartitionEntry) -> [u8; 128] {
    let mut bytes = [0_u8; 128];
    bytes[0..16].copy_from_slice(&entry.partition_type_guid);
    bytes[16..32].copy_from_slice(&entry.unique_partition_guid);
    bytes[32..40].copy_from_slice(&entry.starting_lba.to_le_bytes());
    bytes[40..48].copy_from_slice(&entry.ending_lba.to_le_bytes());
    bytes[48..56].copy_from_slice(&entry.attribute_bits.to_le_bytes());
    let name_bytes: Vec<u8> = entry
        .partition_name
        .as_str()
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let name_len = name_bytes.len().min(72);
    let name_end = 56_usize.saturating_add(name_len);
    if let Some(dst) = bytes.get_mut(56..name_end) {
        dst.copy_from_slice(name_bytes.get(..name_len).unwrap_or(&[]));
    }

    bytes
}

fn write_gpt_header<W: Write>(header: &[u8; 92], writer: &mut W) -> Result<()> {
    writer.write_all(header)?;
    let pad = [0_u8; 512 - 92];
    writer.write_all(&pad)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::types::*;
    use super::*;
    use crate::error::ParttableError;

    fn efi_partition(starting_lba: u64, ending_lba: u64) -> Partition {
        Partition {
            type_guid: EFI_GUID,
            unique_guid: [0xAB; 16],
            starting_lba,
            ending_lba,
            attributes: 0,
            name: "EFI".to_owned(),
        }
    }

    fn request(
        slot: Slot,
        start: Start,
        size: Size,
        type_guid: [u8; 16],
        name: &str,
    ) -> PlacementRequest {
        PlacementRequest {
            slot,
            start,
            size,
            alignment_lba: ALIGN_1_MIB_SECTORS,
            type_guid,
            unique_guid: [0xCD; 16],
            attributes: 0,
            name: name.to_owned(),
        }
    }

    #[test]
    fn align_up_lba_keeps_aligned_value() {
        // ARRANGE
        let lba = ALIGN_1_MIB_SECTORS;

        // ACT
        let result = align_up_lba(lba, ALIGN_1_MIB_SECTORS);

        // ASSERT
        assert_eq!(result, ALIGN_1_MIB_SECTORS);
    }

    #[test]
    fn align_up_lba_rounds_unaligned_value() {
        // ARRANGE
        let lba = ALIGN_1_MIB_SECTORS + 1;

        // ACT
        let result = align_up_lba(lba, ALIGN_1_MIB_SECTORS);

        // ASSERT
        assert_eq!(result, ALIGN_1_MIB_SECTORS * 2);
    }

    #[test]
    fn align_up_lba_keeps_zero() {
        // ARRANGE
        let lba = 0;

        // ACT
        let result = align_up_lba(lba, ALIGN_1_MIB_SECTORS);

        // ASSERT
        assert_eq!(result, 0);
    }

    #[test]
    fn align_up_lba_result_is_always_aligned() {
        // ARRANGE
        let cases = [1_u64, 100, 2047, 2048, 2049, 4095, 4096, 100_000];

        // ACT / ASSERT
        for lba in cases {
            let result = align_up_lba(lba, ALIGN_1_MIB_SECTORS);
            assert!(result.is_multiple_of(ALIGN_1_MIB_SECTORS));
            assert!(result >= lba);
        }
    }

    #[test]
    fn efi_guid_matches_uefi_spec_value() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(
            EFI_GUID,
            [
                0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e,
                0xc9, 0x3b,
            ]
        );
    }

    #[test]
    fn create_table_exposes_usable_lba_range() {
        // ARRANGE / ACT
        let table = Table::create(8 * 2048, 512, [0xCD; 16]).expect("table must be created");

        // ASSERT
        assert!(table.first_usable_lba() > 0);
        assert!(table.last_usable_lba() >= table.first_usable_lba());
    }

    #[test]
    fn partition_persists_through_sequential_write_and_read() {
        // ARRANGE
        let sector_count = 8 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));

        // ACT
        let mut buf = Vec::new();
        table
            .write_primary_to(sector_count, &mut buf)
            .expect("primary write must succeed");
        table
            .write_backup_to(sector_count, &mut buf)
            .expect("backup write must succeed");

        let mut cursor = Cursor::new(buf);
        let reread = GPT::find_from(&mut cursor).expect("GPT must be readable");

        // ASSERT
        let reread_table = Table { inner: reread };
        let partition = reread_table.partition(1).expect("partition must exist");
        assert_eq!(partition.type_guid, EFI_GUID);
        assert_eq!(partition.starting_lba, 2048);
        assert_eq!(partition.ending_lba, 4095);
        assert_eq!(partition.name, "EFI");
    }

    #[test]
    fn sequential_write_matches_gptman_write_into() {
        let sector_count = 8 * 2048;

        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));
        let mut seq_buf = Vec::new();
        table.write_primary_to(sector_count, &mut seq_buf).unwrap();
        table.write_backup_to(sector_count, &mut seq_buf).unwrap();

        let mut disk = Cursor::new(vec![0_u8; 512 * usize::try_from(sector_count).unwrap_or(0)]);
        let mut ref_gpt = GPT::new_from(&mut disk, 512, [0xCD; 16]).expect("gptman create");
        ref_gpt[1] = efi_partition(2048, 4095).into();
        ref_gpt.write_into(&mut disk).expect("gptman write");
        let ref_data = disk.into_inner();

        let primary_start: usize = 512;
        let entries_start: usize = primary_start + 512;
        let entries_end = entries_start.saturating_add(16384);
        assert_eq!(
            seq_buf.get(entries_start..entries_end).unwrap_or(&[]),
            ref_data.get(entries_start..entries_end).unwrap_or(&[]),
            "primary partition entries must match"
        );

        let backup_header_start = ref_data.len().saturating_sub(512);
        let backup_entries_start = backup_header_start.saturating_sub(16384);
        let seq_backup_header_start = seq_buf.len().saturating_sub(512);
        let seq_backup_entries_start = seq_backup_header_start.saturating_sub(16384);
        let seq_backup_end = seq_backup_entries_start.saturating_add(16384);
        let ref_backup_end = backup_entries_start.saturating_add(16384);
        assert_eq!(
            seq_buf
                .get(seq_backup_entries_start..seq_backup_end)
                .unwrap_or(&[]),
            ref_data
                .get(backup_entries_start..ref_backup_end)
                .unwrap_or(&[]),
            "backup partition entries must match"
        );
        let seq_hdr_end = seq_backup_header_start.saturating_add(92);
        let ref_hdr_end = backup_header_start.saturating_add(92);
        assert_eq!(
            seq_buf
                .get(seq_backup_header_start..seq_hdr_end)
                .unwrap_or(&[]),
            ref_data
                .get(backup_header_start..ref_hdr_end)
                .unwrap_or(&[]),
            "backup GPT headers must match"
        );
    }

    #[test]
    fn used_partitions_returns_only_used_entries() {
        // ARRANGE
        let sector_count = 8 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));
        table.set_partition(
            2,
            Partition {
                type_guid: LINUX_FS_GUID,
                unique_guid: [0xBC; 16],
                starting_lba: 4096,
                ending_lba: 8191,
                attributes: 0,
                name: "DATA".to_owned(),
            },
        );

        // ACT
        let used = table.used_partitions();

        // ASSERT
        assert_eq!(used.len(), 2);
        assert!(matches!(used.first(), Some(&(1, _))));
        assert!(matches!(used.get(1), Some(&(2, _))));
    }

    #[test]
    fn highest_used_partition_number_returns_maximum_used_slot() {
        // ARRANGE
        let sector_count = 8 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(2, efi_partition(4096, 8191));
        table.set_partition(7, efi_partition(8192, 12287));

        // ACT
        let highest = table.highest_used_partition_number();

        // ASSERT
        assert_eq!(highest, Some(7));
    }

    #[test]
    fn last_used_ending_lba_returns_farthest_partition_end() {
        // ARRANGE
        let sector_count = 8 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));
        table.set_partition(2, efi_partition(4096, 12287));

        // ACT
        let last_end = table.last_used_ending_lba();

        // ASSERT
        assert_eq!(last_end, Some(12287));
    }

    #[test]
    fn next_free_slot_returns_first_unused_slot() {
        // ARRANGE
        let sector_count = 8 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));
        table.set_partition(3, efi_partition(4096, 8191));

        // ACT
        let next = table.next_free_slot();

        // ASSERT
        assert_eq!(next, Some(2));
    }

    #[test]
    fn place_partition_aligns_first_usable_request() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");

        // ACT
        let placement = table
            .place_partition(
                request(
                    Slot::Exact(1),
                    Start::FirstUsable,
                    Size::Bytes(1024 * 1024),
                    EFI_GUID,
                    "EFI",
                ),
                512,
            )
            .expect("placement must succeed");

        // ASSERT
        assert_eq!(placement.number, 1);
        assert!(
            placement
                .partition
                .starting_lba
                .is_multiple_of(ALIGN_1_MIB_SECTORS)
        );
    }

    #[test]
    fn place_partition_after_partition_uses_previous_end() {
        // ARRANGE
        let sector_count = 32 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        let first = table
            .place_partition(
                request(
                    Slot::Exact(1),
                    Start::FirstUsable,
                    Size::Bytes(1024 * 1024),
                    EFI_GUID,
                    "EFI",
                ),
                512,
            )
            .expect("first placement must succeed");

        // ACT
        let second = table
            .place_partition(
                request(
                    Slot::Exact(2),
                    Start::AfterPartition(first.number),
                    Size::Bytes(1024 * 1024),
                    LINUX_FS_GUID,
                    "STATE",
                ),
                512,
            )
            .expect("second placement must succeed");

        // ASSERT
        assert!(second.partition.starting_lba > first.partition.ending_lba);
        assert!(
            second
                .partition
                .starting_lba
                .is_multiple_of(ALIGN_1_MIB_SECTORS)
        );
    }

    #[test]
    fn place_partition_auto_slot_uses_next_free_slot() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));

        // ACT
        let placement = table
            .place_partition(
                request(
                    Slot::Auto,
                    Start::AfterLastUsed,
                    Size::Bytes(1024 * 1024),
                    LINUX_FS_GUID,
                    "DATA",
                ),
                512,
            )
            .expect("placement must succeed");

        // ASSERT
        assert_eq!(placement.number, 2);
    }

    #[test]
    fn place_partition_fill_to_last_usable_extends_to_table_end() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");

        // ACT
        let placement = table
            .place_partition(
                request(
                    Slot::Exact(1),
                    Start::FirstUsable,
                    Size::FillToLastUsable,
                    LINUX_FS_GUID,
                    "DATA",
                ),
                512,
            )
            .expect("placement must succeed");

        // ASSERT
        assert_eq!(placement.partition.ending_lba, table.last_usable_lba());
    }

    #[test]
    fn place_partition_rejects_overlap() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));

        // ACT
        let result = table.place_partition(
            request(
                Slot::Exact(2),
                Start::AtOrAfter(2048),
                Size::Lbas(10),
                LINUX_FS_GUID,
                "BAD",
            ),
            512,
        );

        // ASSERT
        assert!(matches!(result, Err(ParttableError::InvalidPlacement(_))));
    }

    #[test]
    fn place_partition_rejects_used_exact_slot() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));

        // ACT
        let result = table.place_partition(
            request(
                Slot::Exact(1),
                Start::AfterLastUsed,
                Size::Lbas(10),
                LINUX_FS_GUID,
                "BAD",
            ),
            512,
        );

        // ASSERT
        assert!(matches!(result, Err(ParttableError::InvalidPlacement(_))));
    }

    #[test]
    fn remove_partition_clears_used_slot() {
        // ARRANGE
        let sector_count = 8 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        table.set_partition(1, efi_partition(2048, 4095));

        // ACT
        table
            .remove_partition(1)
            .expect("partition must be removed");

        // ASSERT
        assert!(!table.is_partition_used(1));
        assert!(!table.has_used_partitions());
    }

    #[test]
    fn place_partition_auto_slot_rejects_fully_used_table() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        let max_slots = table
            .inner
            .iter()
            .map(|(number, _)| number)
            .max()
            .expect("table must expose partition slots");
        for slot in 1..=max_slots {
            let start = 2048 + (u64::from(slot) - 1) * ALIGN_1_MIB_SECTORS;
            let end = start + 1023;
            table.set_partition(slot, efi_partition(start, end));
        }

        // ACT
        let result = table.place_partition(
            request(
                Slot::Auto,
                Start::AfterLastUsed,
                Size::Lbas(1),
                LINUX_FS_GUID,
                "OVERFLOW",
            ),
            512,
        );

        // ASSERT
        assert!(
            matches!(result, Err(ParttableError::InvalidPlacement(message)) if message == "no free GPT partition slots")
        );
    }

    #[test]
    fn place_partition_rejects_missing_anchor_partition() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");

        // ACT
        let result = table.place_partition(
            request(
                Slot::Exact(1),
                Start::AfterPartition(9),
                Size::Lbas(1),
                LINUX_FS_GUID,
                "DATA",
            ),
            512,
        );

        // ASSERT
        assert!(
            matches!(result, Err(ParttableError::InvalidPlacement(message)) if message == "cannot place after missing partition 9")
        );
    }

    #[test]
    fn place_partition_rejects_zero_sized_bytes_request() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");

        // ACT
        let result = table.place_partition(
            request(
                Slot::Exact(1),
                Start::FirstUsable,
                Size::Bytes(0),
                LINUX_FS_GUID,
                "EMPTY",
            ),
            512,
        );

        // ASSERT
        assert!(
            matches!(result, Err(ParttableError::InvalidPlacement(message)) if message == "partition size must be greater than zero")
        );
    }

    #[test]
    fn place_partition_rejects_zero_sized_lba_request() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");

        // ACT
        let result = table.place_partition(
            request(
                Slot::Exact(1),
                Start::FirstUsable,
                Size::Lbas(0),
                LINUX_FS_GUID,
                "EMPTY",
            ),
            512,
        );

        // ASSERT
        assert!(
            matches!(result, Err(ParttableError::InvalidPlacement(message)) if message == "partition size must be greater than zero")
        );
    }

    #[test]
    fn place_partition_rejects_start_after_end() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        let invalid_start = table.last_usable_lba() + 1;
        let mut request = request(
            Slot::Exact(1),
            Start::AtOrAfter(invalid_start),
            Size::FillToLastUsable,
            LINUX_FS_GUID,
            "PAST-END",
        );
        request.alignment_lba = 1;

        // ACT
        let result = table.place_partition(request, 512);

        // ASSERT
        assert!(
            matches!(result, Err(ParttableError::InvalidPlacement(message)) if message.contains("start LBA") && message.contains("is after end LBA"))
        );
    }

    #[test]
    fn place_partition_rejects_start_before_first_usable() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        let invalid_start = table.first_usable_lba() - 1;
        let mut request = request(
            Slot::Exact(1),
            Start::AtOrAfter(invalid_start),
            Size::Lbas(1),
            LINUX_FS_GUID,
            "TOO-EARLY",
        );
        request.alignment_lba = 1;

        // ACT
        let result = table.place_partition(request, 512);

        // ASSERT
        assert!(
            matches!(result, Err(ParttableError::InvalidPlacement(message)) if message.contains("starts before first usable LBA"))
        );
    }

    #[test]
    fn place_partition_rejects_end_after_last_usable() {
        // ARRANGE
        let sector_count = 16 * 2048;
        let mut table =
            Table::create(sector_count, 512, [0xCD; 16]).expect("table must be created");
        let invalid_start = table.last_usable_lba();

        // ACT
        let result = table.place_partition(
            request(
                Slot::Exact(1),
                Start::AtOrAfter(invalid_start),
                Size::Lbas(2),
                LINUX_FS_GUID,
                "TOO-LATE",
            ),
            512,
        );

        // ASSERT
        assert!(
            matches!(result, Err(ParttableError::InvalidPlacement(message)) if message.contains("ends after last usable LBA"))
        );
    }
}
