//! LUKS2 binary header parsing and serialization.

use core::str;

use ring::digest::{SHA256, digest};
use ring::rand::{SecureRandom as _, SystemRandom};

use crate::error::{Luks2Error as Error, Result};

const LUKS_MAGIC: [u8; 6] = [0x4c, 0x55, 0x4b, 0x53, 0xba, 0xbe];
const LUKS2_VERSION: u16 = 0x0002;
const BINARY_HEADER_SIZE: usize = 4096;
const DEFAULT_HEADER_SIZE: u64 = 16 * 1024 * 1024;
const CHECKSUM_ALG: &str = "sha256";
const SHA256_LEN: usize = 32;
const CHECKSUM_OFFSET: usize = 376;

const MAGIC_OFFSET: usize = 0;
const MAGIC_END: usize = 6;
const VERSION_OFFSET: usize = 6;
const VERSION_END: usize = 8;
const HEADER_SIZE_OFFSET: usize = 8;
const HEADER_SIZE_END: usize = 16;
const SEQUENCE_ID_OFFSET: usize = 16;
const SEQUENCE_ID_END: usize = 24;
const LABEL_OFFSET: usize = 24;
const LABEL_END: usize = 72;
const CHECKSUM_ALG_OFFSET: usize = 72;
const SALT_OFFSET: usize = 104;
const SALT_END: usize = 168;
const UUID_OFFSET: usize = 168;
const UUID_END: usize = 208;
const SUBSYSTEM_OFFSET: usize = 208;
const SUBSYSTEM_END: usize = 256;
const OFFSET_OFFSET: usize = 256;
const OFFSET_END: usize = 264;

/// On-disk LUKS2 binary header.
#[derive(Debug)]
pub struct Header {
    pub size: u64,
    pub sequence_id: u64,
    pub label: [u8; 48],
    pub salt: [u8; 64],
    pub uuid: [u8; 40],
    pub subsystem: [u8; 48],
    pub checksum: [u8; 64],
}

impl Header {
    /// Creates a new header for formatting a fresh LUKS2 volume.
    pub fn new(uuid_str: &str, label: &str) -> Result<Self> {
        let mut label_buf = [0_u8; 48];
        let label_bytes = label.as_bytes();
        let len = label_bytes.len().min(47);
        copy_prefix(&mut label_buf, label_bytes, len)?;

        let mut uuid_buf = [0_u8; 40];
        let uuid_bytes = uuid_str.as_bytes();
        let len = uuid_bytes.len().min(40);
        copy_prefix(&mut uuid_buf, uuid_bytes, len)?;

        let mut salt = [0_u8; 64];
        SystemRandom::new()
            .fill(&mut salt)
            .map_err(|_error| Error::Rng)?;

        Ok(Self {
            size: DEFAULT_HEADER_SIZE,
            sequence_id: 1,
            label: label_buf,
            salt,
            uuid: uuid_buf,
            subsystem: [0_u8; 48],
            checksum: [0_u8; 64],
        })
    }

    /// Serializes the header to a 4096-byte buffer and computes its checksum.
    pub fn serialize(&mut self, is_primary: bool) -> Result<Vec<u8>> {
        let mut buf = vec![0_u8; BINARY_HEADER_SIZE];

        write_range(&mut buf, MAGIC_OFFSET..MAGIC_END, &LUKS_MAGIC)?;
        write_range(
            &mut buf,
            VERSION_OFFSET..VERSION_END,
            &LUKS2_VERSION.to_be_bytes(),
        )?;
        write_range(
            &mut buf,
            HEADER_SIZE_OFFSET..HEADER_SIZE_END,
            &self.size.to_be_bytes(),
        )?;
        write_range(
            &mut buf,
            SEQUENCE_ID_OFFSET..SEQUENCE_ID_END,
            &self.sequence_id.to_be_bytes(),
        )?;
        write_range(&mut buf, LABEL_OFFSET..LABEL_END, &self.label)?;
        let alg = CHECKSUM_ALG.as_bytes();
        let checksum_alg_end = CHECKSUM_ALG_OFFSET
            .checked_add(alg.len())
            .ok_or_else(|| Error::InvalidField("checksum algorithm range overflow".into()))?;
        write_range(&mut buf, CHECKSUM_ALG_OFFSET..checksum_alg_end, alg)?;
        write_range(&mut buf, SALT_OFFSET..SALT_END, &self.salt)?;
        write_range(&mut buf, UUID_OFFSET..UUID_END, &self.uuid)?;
        write_range(&mut buf, SUBSYSTEM_OFFSET..SUBSYSTEM_END, &self.subsystem)?;
        let offset = if is_primary { 0_u64 } else { self.size };
        write_range(&mut buf, OFFSET_OFFSET..OFFSET_END, &offset.to_be_bytes())?;

        fill_range(&mut buf, CHECKSUM_OFFSET..CHECKSUM_OFFSET + 64, 0)?;

        let hash = digest(&SHA256, &buf);
        write_range(
            &mut buf,
            CHECKSUM_OFFSET..CHECKSUM_OFFSET + SHA256_LEN,
            hash.as_ref(),
        )?;
        copy_prefix(&mut self.checksum, hash.as_ref(), SHA256_LEN)?;

        Ok(buf)
    }

    /// Parses a binary header from a 4096-byte buffer.
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < BINARY_HEADER_SIZE {
            return Err(Error::InvalidField("header too short".into()));
        }

        if read_range(data, MAGIC_OFFSET..MAGIC_END)? != LUKS_MAGIC {
            return Err(Error::InvalidMagic);
        }

        let version = u16::from_be_bytes(read_array::<2>(
            data,
            VERSION_OFFSET..VERSION_END,
            "version",
        )?);
        if version != LUKS2_VERSION {
            return Err(Error::UnsupportedVersion(version));
        }

        let header_size = u64::from_be_bytes(read_array::<8>(
            data,
            HEADER_SIZE_OFFSET..HEADER_SIZE_END,
            "header_size",
        )?);
        let sequence_id = u64::from_be_bytes(read_array::<8>(
            data,
            SEQUENCE_ID_OFFSET..SEQUENCE_ID_END,
            "sequence_id",
        )?);

        let mut label = [0_u8; 48];
        copy_exact(&mut label, read_range(data, LABEL_OFFSET..LABEL_END)?)?;

        let mut salt = [0_u8; 64];
        copy_exact(&mut salt, read_range(data, SALT_OFFSET..SALT_END)?)?;

        let mut uuid = [0_u8; 40];
        copy_exact(&mut uuid, read_range(data, UUID_OFFSET..UUID_END)?)?;

        let mut subsystem = [0_u8; 48];
        copy_exact(
            &mut subsystem,
            read_range(data, SUBSYSTEM_OFFSET..SUBSYSTEM_END)?,
        )?;

        let mut checksum = [0_u8; 64];
        copy_exact(
            &mut checksum,
            read_range(data, CHECKSUM_OFFSET..CHECKSUM_OFFSET + 64)?,
        )?;

        let mut verify_buf = read_range(data, 0..BINARY_HEADER_SIZE)?.to_vec();
        fill_range(&mut verify_buf, CHECKSUM_OFFSET..CHECKSUM_OFFSET + 64, 0)?;
        let computed = digest(&SHA256, &verify_buf);
        if computed.as_ref() != &checksum[..SHA256_LEN] {
            return Err(Error::ChecksumMismatch);
        }

        Ok(Self {
            size: header_size,
            sequence_id,
            label,
            salt,
            uuid,
            subsystem,
            checksum,
        })
    }

    pub fn uuid_str(&self) -> &str {
        let end = self
            .uuid
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(self.uuid.len());
        self.uuid
            .get(..end)
            .and_then(|uuid| str::from_utf8(uuid).ok())
            .unwrap_or("")
    }
}

fn copy_prefix(dst: &mut [u8], src: &[u8], len: usize) -> Result<()> {
    let dst = dst
        .get_mut(..len)
        .ok_or_else(|| Error::InvalidField("destination prefix out of bounds".into()))?;
    let src = src
        .get(..len)
        .ok_or_else(|| Error::InvalidField("source prefix out of bounds".into()))?;
    dst.copy_from_slice(src);

    Ok(())
}

fn copy_exact<const N: usize>(dst: &mut [u8; N], src: &[u8]) -> Result<()> {
    if src.len() != N {
        return Err(Error::InvalidField("slice size mismatch".into()));
    }
    dst.copy_from_slice(src);

    Ok(())
}

fn read_range(data: &[u8], range: core::ops::Range<usize>) -> Result<&[u8]> {
    data.get(range)
        .ok_or_else(|| Error::InvalidField("header slice out of bounds".into()))
}

fn read_array<const N: usize>(
    data: &[u8],
    range: core::ops::Range<usize>,
    field: &str,
) -> Result<[u8; N]> {
    read_range(data, range)?
        .try_into()
        .map_err(|_error| Error::InvalidField(field.into()))
}

fn write_range(buf: &mut [u8], range: core::ops::Range<usize>, src: &[u8]) -> Result<()> {
    let dst = buf
        .get_mut(range)
        .ok_or_else(|| Error::InvalidField("header write out of bounds".into()))?;
    if dst.len() != src.len() {
        return Err(Error::InvalidField("header write size mismatch".into()));
    }
    dst.copy_from_slice(src);

    Ok(())
}

fn fill_range(buf: &mut [u8], range: core::ops::Range<usize>, value: u8) -> Result<()> {
    let dst = buf
        .get_mut(range)
        .ok_or_else(|| Error::InvalidField("header fill out of bounds".into()))?;
    dst.fill(value);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_new() -> Result<()> {
        // ARRANGE
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let label = "test-label";

        // ACT
        let header = Header::new(uuid, label)?;

        // ASSERT
        assert_eq!(header.size, DEFAULT_HEADER_SIZE);
        assert_eq!(header.sequence_id, 1);

        let label_str = std::str::from_utf8(&header.label)
            .unwrap()
            .trim_end_matches('\0');
        assert_eq!(label_str, label);

        assert_eq!(header.uuid_str(), uuid);

        assert!(!header.salt.iter().all(|&b| b == 0));
        Ok(())
    }

    #[test]
    fn header_new_long_label_truncated() -> Result<()> {
        // ARRANGE
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let label = "a".repeat(100);

        // ACT
        let header = Header::new(&uuid, &label)?;

        // ASSERT
        let label_str = std::str::from_utf8(&header.label)
            .unwrap()
            .trim_end_matches('\0');
        assert_eq!(label_str.len(), 47);
        Ok(())
    }

    #[test]
    fn serialize_parse_roundtrip_primary() -> Result<()> {
        // ARRANGE
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let label = "test-label";

        let mut header = Header::new(uuid, label)?;

        // ACT
        let serialized = header.serialize(true)?;

        // ASSERT
        assert_eq!(serialized.len(), BINARY_HEADER_SIZE);

        // ACT
        let parsed = Header::parse(&serialized).unwrap();

        // ASSERT
        assert_eq!(parsed.size, header.size);
        assert_eq!(parsed.sequence_id, header.sequence_id);
        assert_eq!(parsed.uuid_str(), header.uuid_str());
        assert_eq!(parsed.label, header.label);
        assert_eq!(parsed.salt, header.salt);
        Ok(())
    }

    #[test]
    fn serialize_parse_roundtrip_secondary() -> Result<()> {
        // ARRANGE
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let label = "test-label";

        let mut header = Header::new(uuid, label)?;

        // ACT
        let serialized = header.serialize(false)?;

        // ASSERT
        let parsed = Header::parse(&serialized).unwrap();

        assert_eq!(parsed.size, header.size);
        Ok(())
    }

    #[test]
    fn header_magic() -> Result<()> {
        // ARRANGE
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let mut header = Header::new(uuid, "test")?;

        // ACT
        let serialized = header.serialize(true)?;

        // ASSERT
        assert_eq!(&serialized[0..6], &LUKS_MAGIC);
        Ok(())
    }

    #[test]
    fn header_version() -> Result<()> {
        // ARRANGE
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let mut header = Header::new(uuid, "test")?;

        // ACT
        let serialized = header.serialize(true)?;

        // ASSERT
        let version = u16::from_be_bytes([serialized[6], serialized[7]]);
        assert_eq!(version, LUKS2_VERSION);
        Ok(())
    }

    #[test]
    fn header_checksum_validation() -> Result<()> {
        // ARRANGE
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let mut header = Header::new(uuid, "test")?;
        let mut serialized = header.serialize(true)?;

        // ACT & ASSERT
        let result = Header::parse(&serialized);
        assert!(result.is_ok());

        // ARRANGE
        serialized[100] ^= 0xFF;

        // ACT & ASSERT
        let result = Header::parse(&serialized);
        assert!(matches!(result, Err(Error::ChecksumMismatch)));
        Ok(())
    }

    #[test]
    fn header_checksum_corruption() -> Result<()> {
        // ARRANGE
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let mut header = Header::new(uuid, "test")?;
        let mut serialized = header.serialize(true)?;

        let checksum_start = CHECKSUM_OFFSET;
        serialized[checksum_start] ^= 0xFF;

        // ACT
        let result = Header::parse(&serialized);

        // ASSERT
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn parse_invalid_magic() -> Result<()> {
        // ARRANGE
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let mut header = Header::new(uuid, "test")?;
        let mut serialized = header.serialize(true)?;

        serialized[0] = 0x00;
        serialized[1] = 0x00;

        // ACT
        let result = Header::parse(&serialized);

        // ASSERT
        assert!(matches!(result, Err(Error::InvalidMagic)));
        Ok(())
    }

    #[test]
    fn parse_unsupported_version() -> Result<()> {
        // ARRANGE
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let mut header = Header::new(uuid, "test")?;
        let mut serialized = header.serialize(true)?;

        serialized[6..8].copy_from_slice(&1u16.to_be_bytes());

        // ACT
        let result = Header::parse(&serialized);

        // ASSERT
        assert!(matches!(result, Err(Error::UnsupportedVersion(1))));
        Ok(())
    }

    #[test]
    fn parse_too_short() {
        // ARRANGE
        let data = vec![0u8; 100];

        // ACT
        let result = Header::parse(&data);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn uuid_str_various_lengths() -> Result<()> {
        // ARRANGE & ACT
        let header = Header::new("abc", "test")?;
        let long_uuid = "a".repeat(40);
        let long_header = Header::new(&long_uuid, "test")?;
        let very_long_uuid = "a".repeat(50);
        let very_long_header = Header::new(&very_long_uuid, "test")?;

        // ASSERT
        assert_eq!(header.uuid_str(), "abc");
        assert_eq!(long_header.uuid_str(), long_uuid);
        assert_eq!(very_long_header.uuid_str().len(), 40);
        Ok(())
    }

    #[test]
    fn header_offset_field() -> Result<()> {
        // ARRANGE
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let mut header = Header::new(uuid, "test")?;

        // ACT
        let serialized_primary = header.serialize(true)?;

        // ASSERT
        let offset_primary = u64::from_be_bytes([
            serialized_primary[256],
            serialized_primary[257],
            serialized_primary[258],
            serialized_primary[259],
            serialized_primary[260],
            serialized_primary[261],
            serialized_primary[262],
            serialized_primary[263],
        ]);
        assert_eq!(offset_primary, 0);

        // ACT
        let serialized_secondary = header.serialize(false)?;

        // ASSERT
        let offset_secondary = u64::from_be_bytes([
            serialized_secondary[256],
            serialized_secondary[257],
            serialized_secondary[258],
            serialized_secondary[259],
            serialized_secondary[260],
            serialized_secondary[261],
            serialized_secondary[262],
            serialized_secondary[263],
        ]);
        assert_eq!(offset_secondary, DEFAULT_HEADER_SIZE);
        Ok(())
    }

    #[test]
    fn different_headers_different_checksums() -> Result<()> {
        // ARRANGE
        let uuid1 = "12345678-1234-1234-1234-123456789abc";
        let uuid2 = "87654321-4321-4321-4321-cba987654321";

        let mut header1 = Header::new(uuid1, "test1")?;
        let mut header2 = Header::new(uuid2, "test2")?;

        // ACT
        let serialized1 = header1.serialize(true)?;
        let serialized2 = header2.serialize(true)?;

        // ASSERT
        let checksum1 = &serialized1[CHECKSUM_OFFSET..CHECKSUM_OFFSET + SHA256_LEN];
        let checksum2 = &serialized2[CHECKSUM_OFFSET..CHECKSUM_OFFSET + SHA256_LEN];

        assert_ne!(checksum1, checksum2);
        Ok(())
    }

    #[test]
    fn checksum_algorithm_field() -> Result<()> {
        // ARRANGE
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let mut header = Header::new(uuid, "test")?;

        // ACT
        let serialized = header.serialize(true)?;

        // ASSERT
        let alg_bytes = &serialized[72..104];
        let alg_str = std::str::from_utf8(alg_bytes)
            .unwrap()
            .trim_end_matches('\0');
        assert_eq!(alg_str, CHECKSUM_ALG);
        Ok(())
    }

    #[test]
    fn write_range_rejects_mismatched_lengths() {
        // ARRANGE
        let mut buffer = [0_u8; 4];

        // ACT
        let result = write_range(&mut buffer, 0..2, &[1, 2, 3]);

        // ASSERT
        assert!(
            matches!(result, Err(Error::InvalidField(field)) if field == "header write size mismatch")
        );
    }

    #[test]
    fn copy_exact_rejects_wrong_length() {
        // ARRANGE
        let mut dst = [0_u8; 4];

        // ACT
        let result = copy_exact(&mut dst, &[1, 2]);

        // ASSERT
        assert!(
            matches!(result, Err(Error::InvalidField(field)) if field == "slice size mismatch")
        );
    }

    #[test]
    fn read_range_rejects_out_of_bounds_range() {
        // ACT
        let result = read_range(&[0_u8; 2], 0..3);

        // ASSERT
        assert!(
            matches!(result, Err(Error::InvalidField(field)) if field == "header slice out of bounds")
        );
    }

    #[test]
    fn fill_range_rejects_out_of_bounds_range() {
        // ARRANGE
        let mut dst = [0_u8; 2];

        // ACT
        let result = fill_range(&mut dst, 0..3, 0);

        // ASSERT
        assert!(
            matches!(result, Err(Error::InvalidField(field)) if field == "header fill out of bounds")
        );
    }
}
