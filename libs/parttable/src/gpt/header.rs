//! GPT header wire format: parsing and encoding with CRC validation.

use core::ops::Range;

use crate::error::{ParttableError, Result};

/// Size of the GPT header in bytes (the CRC-protected region).
pub(crate) const GPT_HEADER_SIZE: usize = 92;

// GPT header field offsets within the 92-byte header.
const HDR_SIGNATURE: Range<usize> = 0..8;
const HDR_REVISION: Range<usize> = 8..12;
const HDR_SIZE: Range<usize> = 12..16;
const HDR_CRC: Range<usize> = 16..20;
const HDR_CURRENT_LBA: Range<usize> = 24..32;
const HDR_BACKUP_LBA: Range<usize> = 32..40;
const HDR_FIRST_USABLE: Range<usize> = 40..48;
const HDR_LAST_USABLE: Range<usize> = 48..56;
const HDR_DISK_GUID: Range<usize> = 56..72;
const HDR_ENTRIES_LBA: Range<usize> = 72..80;
const HDR_ENTRIES_COUNT: Range<usize> = 80..84;
const HDR_ENTRY_SIZE: Range<usize> = 84..88;
const HDR_ENTRIES_CRC: Range<usize> = 88..92;

/// The decoded fields of a GPT header.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GptHeader {
    /// LBA where the primary partition entries array starts.
    pub entries_lba: u64,
    /// First LBA usable by partitions.
    pub first_usable_lba: u64,
    /// Last LBA usable by partitions.
    pub last_usable_lba: u64,
    /// GUID identifying the disk.
    pub disk_guid: [u8; 16],
    /// Number of partition entry slots.
    pub entries_count: u32,
    /// Size in bytes of each partition entry.
    pub entries_size: u32,
    /// CRC32 of the partition entries array.
    pub entries_crc: u32,
}

impl GptHeader {
    /// Parses and validates a 512-byte GPT header sector.
    ///
    /// # Errors
    ///
    /// Returns an error when the signature, revision, size, or header CRC is invalid.
    pub(crate) fn parse(sector: &[u8; 512]) -> Result<Self> {
        let signature = field(sector, HDR_SIGNATURE)?;
        if signature != b"EFI PART" {
            return Err(ParttableError::Gpt("invalid GPT signature".to_owned()));
        }

        let revision = le_u32(sector, HDR_REVISION)?;
        if revision < 0x0001_0000 {
            return Err(ParttableError::Gpt("unsupported GPT revision".to_owned()));
        }

        let header_size = le_u32(sector, HDR_SIZE)?;
        if header_size < u32::try_from(GPT_HEADER_SIZE).unwrap_or(0) {
            return Err(ParttableError::Gpt("GPT header too small".to_owned()));
        }

        let header_crc = le_u32(sector, HDR_CRC)?;
        let mut crc_buffer = [0_u8; GPT_HEADER_SIZE];
        crc_buffer.copy_from_slice(field(sector, 0..GPT_HEADER_SIZE)?);
        if let Some(crc) = crc_buffer.get_mut(HDR_CRC) {
            crc.copy_from_slice(&[0_u8; 4]);
        }
        if crc32fast::hash(&crc_buffer) != header_crc {
            return Err(ParttableError::Gpt("GPT header CRC mismatch".to_owned()));
        }

        let disk_guid: [u8; 16] = field(sector, HDR_DISK_GUID)?
            .try_into()
            .map_err(|_err| ParttableError::Gpt("GPT header disk GUID truncated".to_owned()))?;

        Ok(Self {
            entries_lba: le_u64(sector, HDR_ENTRIES_LBA)?,
            first_usable_lba: le_u64(sector, HDR_FIRST_USABLE)?,
            last_usable_lba: le_u64(sector, HDR_LAST_USABLE)?,
            disk_guid,
            entries_count: le_u32(sector, HDR_ENTRIES_COUNT)?,
            entries_size: le_u32(sector, HDR_ENTRY_SIZE)?,
            entries_crc: le_u32(sector, HDR_ENTRIES_CRC)?,
        })
    }

    /// Encodes a full 512-byte GPT header sector for the primary or backup copy.
    #[must_use]
    pub(crate) fn encode(
        &self,
        backup: bool,
        sector_count: u64,
        sector_size: u64,
        entries_crc: u32,
    ) -> [u8; 512] {
        let mut header = [0_u8; GPT_HEADER_SIZE];
        put(&mut header, HDR_SIGNATURE, b"EFI PART");
        put(&mut header, HDR_REVISION, &0x0001_0000_u32.to_le_bytes());
        put(
            &mut header,
            HDR_SIZE,
            &u32::try_from(GPT_HEADER_SIZE).unwrap_or(0).to_le_bytes(),
        );

        let (current_lba, backup_lba) = if backup {
            (sector_count.saturating_sub(1), 1_u64)
        } else {
            (1_u64, sector_count.saturating_sub(1))
        };
        put(&mut header, HDR_CURRENT_LBA, &current_lba.to_le_bytes());
        put(&mut header, HDR_BACKUP_LBA, &backup_lba.to_le_bytes());
        put(
            &mut header,
            HDR_FIRST_USABLE,
            &self.first_usable_lba.to_le_bytes(),
        );
        put(
            &mut header,
            HDR_LAST_USABLE,
            &self.last_usable_lba.to_le_bytes(),
        );
        put(&mut header, HDR_DISK_GUID, &self.disk_guid);

        let entries_lba = if backup {
            sector_count.saturating_sub(1).saturating_sub(
                u64::from(self.entries_count)
                    .saturating_mul(u64::from(self.entries_size))
                    .div_ceil(sector_size),
            )
        } else {
            self.entries_lba
        };
        put(&mut header, HDR_ENTRIES_LBA, &entries_lba.to_le_bytes());
        put(
            &mut header,
            HDR_ENTRIES_COUNT,
            &self.entries_count.to_le_bytes(),
        );
        put(
            &mut header,
            HDR_ENTRY_SIZE,
            &self.entries_size.to_le_bytes(),
        );
        put(&mut header, HDR_ENTRIES_CRC, &entries_crc.to_le_bytes());

        let header_crc = crc32fast::hash(&header);
        put(&mut header, HDR_CRC, &header_crc.to_le_bytes());

        let mut sector = [0_u8; 512];
        if let Some(dst) = sector.get_mut(..GPT_HEADER_SIZE) {
            dst.copy_from_slice(&header);
        }

        sector
    }
}

fn field(sector: &[u8; 512], range: core::ops::Range<usize>) -> Result<&[u8]> {
    sector
        .get(range)
        .ok_or_else(|| ParttableError::Gpt("GPT header truncated".to_owned()))
}

fn le_u32(sector: &[u8; 512], range: core::ops::Range<usize>) -> Result<u32> {
    let bytes: [u8; 4] = field(sector, range)?
        .try_into()
        .map_err(|_err| ParttableError::Gpt("GPT header field truncated".to_owned()))?;

    Ok(u32::from_le_bytes(bytes))
}

fn le_u64(sector: &[u8; 512], range: core::ops::Range<usize>) -> Result<u64> {
    let bytes: [u8; 8] = field(sector, range)?
        .try_into()
        .map_err(|_err| ParttableError::Gpt("GPT header field truncated".to_owned()))?;

    Ok(u64::from_le_bytes(bytes))
}

fn put(header: &mut [u8; GPT_HEADER_SIZE], range: core::ops::Range<usize>, bytes: &[u8]) {
    if let Some(dst) = header.get_mut(range) {
        dst.copy_from_slice(bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_header() -> GptHeader {
        GptHeader {
            entries_lba: 2,
            first_usable_lba: 34,
            last_usable_lba: 16_318,
            disk_guid: [0xCD; 16],
            entries_count: 128,
            entries_size: 128,
            entries_crc: 0xDEAD_BEEF,
        }
    }

    #[test]
    fn encode_then_parse_round_trips() {
        // ARRANGE
        let header = sample_header();

        // ACT
        let sector = header.encode(false, 16_384, 512, 0xDEAD_BEEF);
        let decoded = GptHeader::parse(&sector).expect("header must parse");

        // ASSERT
        assert_eq!(decoded.entries_lba, header.entries_lba);
        assert_eq!(decoded.first_usable_lba, header.first_usable_lba);
        assert_eq!(decoded.last_usable_lba, header.last_usable_lba);
        assert_eq!(decoded.disk_guid, header.disk_guid);
        assert_eq!(decoded.entries_count, header.entries_count);
        assert_eq!(decoded.entries_size, header.entries_size);
        assert_eq!(decoded.entries_crc, header.entries_crc);
    }

    #[test]
    fn parse_rejects_corrupted_header_crc() {
        // ARRANGE
        let header = sample_header();
        let mut sector = header.encode(false, 16_384, 512, 0xDEAD_BEEF);

        // ACT
        if let Some(byte) = sector.get_mut(40) {
            *byte = byte.wrapping_add(1);
        }
        let result = GptHeader::parse(&sector);

        // ASSERT
        assert!(
            matches!(result, Err(ParttableError::Gpt(message)) if message == "GPT header CRC mismatch")
        );
    }

    #[test]
    fn parse_rejects_invalid_signature() {
        // ARRANGE
        let header = sample_header();
        let mut sector = header.encode(false, 16_384, 512, 0xDEAD_BEEF);
        sector[0] = b'X';

        // ACT
        let result = GptHeader::parse(&sector);

        // ASSERT
        assert!(
            matches!(result, Err(ParttableError::Gpt(message)) if message == "invalid GPT signature")
        );
    }

    #[test]
    fn backup_encode_places_header_at_end_of_disk() {
        // ARRANGE
        let header = sample_header();
        let sector_count = 16_384;

        // ACT
        let sector = header.encode(true, sector_count, 512, 0xDEAD_BEEF);

        // ASSERT
        let current = u64::from_le_bytes(sector[24..32].try_into().expect("slice"));
        let backup = u64::from_le_bytes(sector[32..40].try_into().expect("slice"));
        assert_eq!(current, sector_count.saturating_sub(1));
        assert_eq!(backup, 1);
    }
}
