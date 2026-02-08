//! LUKS2 binary header parsing and serialization.
//!
//! The binary header occupies the first 4096 bytes of the device and contains
//! magic bytes, version, sizes, UUID, salt, and a SHA-256 integrity checksum.
//! A secondary copy follows the JSON + keyslot area for redundancy.

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
        let len = uuid_bytes.len().min(39);
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

        // Magic (6 bytes)
        buf[0..6].copy_from_slice(&LUKS_MAGIC);
        // Version (2 bytes, big-endian)
        buf[6..8].copy_from_slice(&LUKS2_VERSION.to_be_bytes());
        // Header size (8 bytes, big-endian)
        buf[8..16].copy_from_slice(&self.header_size.to_be_bytes());
        // Sequence ID (8 bytes, big-endian)
        buf[16..24].copy_from_slice(&self.sequence_id.to_be_bytes());
        // Label (48 bytes)
        buf[24..72].copy_from_slice(&self.label);
        // Checksum algorithm (32 bytes, null-terminated)
        let alg = CHECKSUM_ALG.as_bytes();
        buf[72..72 + alg.len()].copy_from_slice(alg);
        // Salt (64 bytes)
        buf[104..168].copy_from_slice(&self.salt);
        // UUID (40 bytes)
        buf[168..208].copy_from_slice(&self.uuid);
        // Subsystem (48 bytes)
        buf[208..256].copy_from_slice(&self.subsystem);
        // Header offset (8 bytes, big-endian) — 0 for primary, header_size for secondary
        let offset = if is_primary { 0u64 } else { self.header_size };
        buf[256..264].copy_from_slice(&offset.to_be_bytes());

        // Zero checksum field before computing
        buf[CHECKSUM_OFFSET..CHECKSUM_OFFSET + 64].fill(0);

        // Compute SHA-256 over entire 4096-byte header
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

        // Validate magic
        if data[0..6] != LUKS_MAGIC {
            return Err(Error::InvalidMagic);
        }

        // Validate version
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

        // Bytes 256..264 contain the header_offset field (0 for primary,
        // header_size for secondary). Parsed for checksum verification but
        // not stored — serialize() derives the value from `is_primary`.

        let mut checksum = [0u8; 64];
        checksum.copy_from_slice(&data[CHECKSUM_OFFSET..CHECKSUM_OFFSET + 64]);

        // Verify checksum: zero the checksum field, hash, compare
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

    /// Returns the UUID as a string, trimmed of null bytes.
    pub fn uuid_str(&self) -> &str {
        let end = self
            .uuid
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.uuid.len());
        std::str::from_utf8(&self.uuid[..end]).unwrap_or("")
    }
}
