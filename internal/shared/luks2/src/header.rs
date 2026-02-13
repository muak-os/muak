//! LUKS2 binary header parsing and serialization.

use ring::digest::SHA256;

use crate::constants::{
    BINARY_HEADER_SIZE, CHECKSUM_ALG, CHECKSUM_OFFSET, DEFAULT_HEADER_SIZE, LUKS_MAGIC,
    LUKS2_VERSION, SHA256_LEN,
};
use crate::error::{Error, Result};

/// On-disk LUKS2 binary header.
#[derive(Debug)]
pub struct Header {
    pub header_size: u64,
    pub sequence_id: u64,
    pub label: [u8; 48],
    pub salt: [u8; 64],
    pub uuid: [u8; 40],
    pub subsystem: [u8; 48],
    pub checksum: [u8; 64],
}

impl Header {
    /// Creates a new header for formatting a fresh LUKS2 volume.
    pub fn new(uuid_str: &str, label: &str) -> Self {
        let mut label_buf = [0u8; 48];
        let label_bytes = label.as_bytes();
        let len = label_bytes.len().min(47);
        label_buf[..len].copy_from_slice(&label_bytes[..len]);

        let mut uuid_buf = [0u8; 40];
        let uuid_bytes = uuid_str.as_bytes();
        let len = uuid_bytes.len().min(40);
        uuid_buf[..len].copy_from_slice(&uuid_bytes[..len]);

        let mut salt = [0u8; 64];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut salt)
            .expect("failed to generate random salt");

        Self {
            header_size: DEFAULT_HEADER_SIZE,
            sequence_id: 1,
            label: label_buf,
            salt,
            uuid: uuid_buf,
            subsystem: [0u8; 48],
            checksum: [0u8; 64],
        }
    }

    /// Serializes the header to a 4096-byte buffer and computes its checksum.
    pub fn serialize(&mut self, is_primary: bool) -> Vec<u8> {
        let mut buf = vec![0u8; BINARY_HEADER_SIZE];

        buf[0..6].copy_from_slice(&LUKS_MAGIC);
        buf[6..8].copy_from_slice(&LUKS2_VERSION.to_be_bytes());
        buf[8..16].copy_from_slice(&self.header_size.to_be_bytes());
        buf[16..24].copy_from_slice(&self.sequence_id.to_be_bytes());
        buf[24..72].copy_from_slice(&self.label);
        let alg = CHECKSUM_ALG.as_bytes();
        buf[72..72 + alg.len()].copy_from_slice(alg);
        buf[104..168].copy_from_slice(&self.salt);
        buf[168..208].copy_from_slice(&self.uuid);
        buf[208..256].copy_from_slice(&self.subsystem);
        let offset = if is_primary { 0u64 } else { self.header_size };
        buf[256..264].copy_from_slice(&offset.to_be_bytes());

        buf[CHECKSUM_OFFSET..CHECKSUM_OFFSET + 64].fill(0);

        let hash = ring::digest::digest(&SHA256, &buf);
        buf[CHECKSUM_OFFSET..CHECKSUM_OFFSET + SHA256_LEN].copy_from_slice(hash.as_ref());
        self.checksum[..SHA256_LEN].copy_from_slice(hash.as_ref());

        buf
    }

    /// Parses a binary header from a 4096-byte buffer.
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < BINARY_HEADER_SIZE {
            return Err(Error::InvalidField("header too short".into()));
        }

        if data[0..6] != LUKS_MAGIC {
            return Err(Error::InvalidMagic);
        }

        let version = u16::from_be_bytes([data[6], data[7]]);
        if version != LUKS2_VERSION {
            return Err(Error::UnsupportedVersion(version));
        }

        let header_size = u64::from_be_bytes(
            data[8..16]
                .try_into()
                .map_err(|_| Error::InvalidField("header_size".into()))?,
        );
        let sequence_id = u64::from_be_bytes(
            data[16..24]
                .try_into()
                .map_err(|_| Error::InvalidField("sequence_id".into()))?,
        );

        let mut label = [0u8; 48];
        label.copy_from_slice(&data[24..72]);

        let mut salt = [0u8; 64];
        salt.copy_from_slice(&data[104..168]);

        let mut uuid = [0u8; 40];
        uuid.copy_from_slice(&data[168..208]);

        let mut subsystem = [0u8; 48];
        subsystem.copy_from_slice(&data[208..256]);

        let mut checksum = [0u8; 64];
        checksum.copy_from_slice(&data[CHECKSUM_OFFSET..CHECKSUM_OFFSET + 64]);

        let mut verify_buf = data[..BINARY_HEADER_SIZE].to_vec();
        verify_buf[CHECKSUM_OFFSET..CHECKSUM_OFFSET + 64].fill(0);
        let computed = ring::digest::digest(&SHA256, &verify_buf);
        if computed.as_ref() != &checksum[..SHA256_LEN] {
            return Err(Error::ChecksumMismatch);
        }

        Ok(Self {
            header_size,
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
            .position(|&b| b == 0)
            .unwrap_or(self.uuid.len());
        std::str::from_utf8(&self.uuid[..end]).unwrap_or("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_new() {
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let label = "test-label";

        let header = Header::new(uuid, label);

        assert_eq!(header.header_size, DEFAULT_HEADER_SIZE);
        assert_eq!(header.sequence_id, 1);

        let label_str = std::str::from_utf8(&header.label)
            .unwrap()
            .trim_end_matches('\0');
        assert_eq!(label_str, label);

        assert_eq!(header.uuid_str(), uuid);

        assert!(!header.salt.iter().all(|&b| b == 0));
    }

    #[test]
    fn test_header_new_long_label_truncated() {
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let label = "a".repeat(100);

        let header = Header::new(&uuid, &label);

        let label_str = std::str::from_utf8(&header.label)
            .unwrap()
            .trim_end_matches('\0');
        assert_eq!(label_str.len(), 47);
    }

    #[test]
    fn test_serialize_parse_roundtrip_primary() {
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let label = "test-label";

        let mut header = Header::new(uuid, label);
        let serialized = header.serialize(true);

        assert_eq!(serialized.len(), BINARY_HEADER_SIZE);

        let parsed = Header::parse(&serialized).unwrap();

        assert_eq!(parsed.header_size, header.header_size);
        assert_eq!(parsed.sequence_id, header.sequence_id);
        assert_eq!(parsed.uuid_str(), header.uuid_str());
        assert_eq!(parsed.label, header.label);
        assert_eq!(parsed.salt, header.salt);
    }

    #[test]
    fn test_serialize_parse_roundtrip_secondary() {
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let label = "test-label";

        let mut header = Header::new(uuid, label);
        let serialized = header.serialize(false);

        let parsed = Header::parse(&serialized).unwrap();

        assert_eq!(parsed.header_size, header.header_size);
    }

    #[test]
    fn test_header_magic() {
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let mut header = Header::new(uuid, "test");
        let serialized = header.serialize(true);

        assert_eq!(&serialized[0..6], &LUKS_MAGIC);
    }

    #[test]
    fn test_header_version() {
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let mut header = Header::new(uuid, "test");
        let serialized = header.serialize(true);

        let version = u16::from_be_bytes([serialized[6], serialized[7]]);
        assert_eq!(version, LUKS2_VERSION);
    }

    #[test]
    fn test_header_checksum_validation() {
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let mut header = Header::new(uuid, "test");
        let mut serialized = header.serialize(true);

        let result = Header::parse(&serialized);
        assert!(result.is_ok());

        serialized[100] ^= 0xFF;

        let result = Header::parse(&serialized);
        assert!(matches!(result, Err(Error::ChecksumMismatch)));
    }

    #[test]
    fn test_header_checksum_corruption() {
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let mut header = Header::new(uuid, "test");
        let mut serialized = header.serialize(true);

        let checksum_start = CHECKSUM_OFFSET;
        serialized[checksum_start] ^= 0xFF;

        let result = Header::parse(&serialized);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_invalid_magic() {
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let mut header = Header::new(uuid, "test");
        let mut serialized = header.serialize(true);

        serialized[0] = 0x00;
        serialized[1] = 0x00;

        let result = Header::parse(&serialized);
        assert!(matches!(result, Err(Error::InvalidMagic)));
    }

    #[test]
    fn test_parse_unsupported_version() {
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let mut header = Header::new(uuid, "test");
        let mut serialized = header.serialize(true);

        serialized[6..8].copy_from_slice(&1u16.to_be_bytes());

        let result = Header::parse(&serialized);
        assert!(matches!(result, Err(Error::UnsupportedVersion(1))));
    }

    #[test]
    fn test_parse_too_short() {
        let data = vec![0u8; 100];

        let result = Header::parse(&data);
        assert!(result.is_err());
    }

    #[test]
    fn test_uuid_str_various_lengths() {
        let mut header = Header::new("abc", "test");
        assert_eq!(header.uuid_str(), "abc");

        let long_uuid = "a".repeat(40);
        header = Header::new(&long_uuid, "test");
        assert_eq!(header.uuid_str(), long_uuid);

        let very_long_uuid = "a".repeat(50);
        header = Header::new(&very_long_uuid, "test");
        assert_eq!(header.uuid_str().len(), 40);
    }

    #[test]
    fn test_header_offset_field() {
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let mut header = Header::new(uuid, "test");

        let serialized_primary = header.serialize(true);
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

        let serialized_secondary = header.serialize(false);
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
    }

    #[test]
    fn test_different_headers_different_checksums() {
        let uuid1 = "12345678-1234-1234-1234-123456789abc";
        let uuid2 = "87654321-4321-4321-4321-cba987654321";

        let mut header1 = Header::new(uuid1, "test1");
        let mut header2 = Header::new(uuid2, "test2");

        let serialized1 = header1.serialize(true);
        let serialized2 = header2.serialize(true);

        let checksum1 = &serialized1[CHECKSUM_OFFSET..CHECKSUM_OFFSET + SHA256_LEN];
        let checksum2 = &serialized2[CHECKSUM_OFFSET..CHECKSUM_OFFSET + SHA256_LEN];

        assert_ne!(checksum1, checksum2);
    }

    #[test]
    fn test_checksum_algorithm_field() {
        let uuid = "12345678-1234-1234-1234-123456789abc";
        let mut header = Header::new(uuid, "test");
        let serialized = header.serialize(true);

        let alg_bytes = &serialized[72..104];
        let alg_str = std::str::from_utf8(alg_bytes)
            .unwrap()
            .trim_end_matches('\0');
        assert_eq!(alg_str, CHECKSUM_ALG);
    }
}
