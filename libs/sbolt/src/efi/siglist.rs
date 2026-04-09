//! EFI Signature List structures

use uefi::Guid;

use super::EFI_CERT_X509_GUID;
use crate::{Error, Result};

pub const SIGNATURE_LIST_HEADER_SIZE: usize = 28;
pub const SIGNATURE_DATA_HEADER_SIZE: usize = 16;

/// A signature database containing multiple signature lists
#[derive(Default)]
pub struct SignatureDatabase {
    lists: Vec<Vec<u8>>,
}

impl SignatureDatabase {
    /// Create a new empty signature database
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse a signature database from raw bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let lists = parse_signature_lists(data)?;
        Ok(Self { lists })
    }

    /// Add an X.509 certificate to the database
    pub fn add_x509(&mut self, owner: &Guid, cert_der: &[u8]) {
        self.lists.push(build_x509_siglist(owner, cert_der));
    }

    /// Serialize the database to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        self.lists.iter().flatten().copied().collect()
    }

    /// Get the number of signature lists
    pub fn len(&self) -> usize {
        self.lists.len()
    }

    /// Check if the database is empty
    pub fn is_empty(&self) -> bool {
        self.lists.is_empty()
    }
}

fn parse_signature_lists(data: &[u8]) -> Result<Vec<Vec<u8>>> {
    let mut lists = Vec::new();
    let mut offset = 0;
    while offset + SIGNATURE_LIST_HEADER_SIZE <= data.len() {
        let list_size = u32::from_le_bytes(
            data[offset + 16..offset + 20]
                .try_into()
                .map_err(|_| Error::EfiVar("read signature list size".into()))?,
        ) as usize;
        if list_size < SIGNATURE_LIST_HEADER_SIZE || offset + list_size > data.len() {
            return Err(Error::EfiVar("invalid signature list size".into()));
        }
        lists.push(data[offset..offset + list_size].to_vec());
        offset += list_size;
    }
    Ok(lists)
}

/// Build an EFI_SIGNATURE_LIST containing a single X.509 certificate
pub fn build_x509_siglist(owner_guid: &Guid, cert_der: &[u8]) -> Vec<u8> {
    let signature_size = SIGNATURE_DATA_HEADER_SIZE + cert_der.len();
    let list_size = SIGNATURE_LIST_HEADER_SIZE + signature_size;

    let mut buf = Vec::with_capacity(list_size);

    buf.extend_from_slice(&EFI_CERT_X509_GUID.to_bytes());
    buf.extend_from_slice(&(list_size as u32).to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&(signature_size as u32).to_le_bytes());

    buf.extend_from_slice(&owner_guid.to_bytes());
    buf.extend_from_slice(cert_der);

    buf
}

#[cfg(test)]
mod tests {
    use uefi::guid;

    use super::*;

    const TEST_OWNER: Guid = guid!("12345678-1234-1234-1234-123456789abc");
    const FAKE_CERT: &[u8] = b"fake-x509-cert-der-data";

    #[test]
    fn build_x509_siglist_layout() {
        // ACT
        let siglist = build_x509_siglist(&TEST_OWNER, FAKE_CERT);

        // ASSERT
        let expected_sig_size = SIGNATURE_DATA_HEADER_SIZE + FAKE_CERT.len();
        let expected_list_size = SIGNATURE_LIST_HEADER_SIZE + expected_sig_size;
        assert_eq!(siglist.len(), expected_list_size);

        assert_eq!(&siglist[0..16], &EFI_CERT_X509_GUID.to_bytes());

        let list_size = u32::from_le_bytes(siglist[16..20].try_into().expect("4 bytes"));
        assert_eq!(list_size as usize, expected_list_size);

        let header_size = u32::from_le_bytes(siglist[20..24].try_into().expect("4 bytes"));
        assert_eq!(header_size, 0);

        let sig_size = u32::from_le_bytes(siglist[24..28].try_into().expect("4 bytes"));
        assert_eq!(sig_size as usize, expected_sig_size);

        assert_eq!(&siglist[28..44], &TEST_OWNER.to_bytes());

        assert_eq!(&siglist[44..], FAKE_CERT);
    }

    #[test]
    fn roundtrip_build_then_parse() {
        // ARRANGE
        let siglist = build_x509_siglist(&TEST_OWNER, FAKE_CERT);

        // ACT
        let db = SignatureDatabase::from_bytes(&siglist).expect("parse siglist");

        // ASSERT
        assert_eq!(db.len(), 1);
        assert!(!db.is_empty());
        assert_eq!(db.to_bytes(), siglist);
    }

    #[test]
    fn empty_database() {
        // ACT
        let db = SignatureDatabase::new();

        // ASSERT
        assert!(db.is_empty());
        assert_eq!(db.len(), 0);
        assert!(db.to_bytes().is_empty());
    }

    #[test]
    fn add_x509_increments_count() {
        // ARRANGE
        let mut db = SignatureDatabase::new();

        // ACT
        db.add_x509(&TEST_OWNER, FAKE_CERT);

        // ASSERT
        assert_eq!(db.len(), 1);

        // ACT
        db.add_x509(&TEST_OWNER, b"second-cert");

        // ASSERT
        assert_eq!(db.len(), 2);
    }

    #[test]
    fn multi_cert_roundtrip() {
        // ARRANGE
        let cert_a = b"cert-alpha";
        let cert_b = b"cert-bravo";

        let mut db = SignatureDatabase::new();
        db.add_x509(&TEST_OWNER, cert_a);
        db.add_x509(&TEST_OWNER, cert_b);

        // ACT
        let bytes = db.to_bytes();
        let db2 = SignatureDatabase::from_bytes(&bytes).expect("parse multi-cert db");

        // ASSERT
        assert_eq!(db2.len(), 2);
        assert_eq!(db2.to_bytes(), bytes);
    }

    #[test]
    fn from_bytes_rejects_truncated_header() {
        // ARRANGE
        let short = vec![0u8; SIGNATURE_LIST_HEADER_SIZE - 1];

        // ACT
        let db = SignatureDatabase::from_bytes(&short).expect("parse short data");

        // ASSERT
        assert!(db.is_empty());
    }

    #[test]
    fn from_bytes_rejects_invalid_list_size_too_small() {
        // ARRANGE
        let mut data = vec![0u8; SIGNATURE_LIST_HEADER_SIZE];
        let bad_size = (SIGNATURE_LIST_HEADER_SIZE - 1) as u32;
        data[16..20].copy_from_slice(&bad_size.to_le_bytes());

        // ACT
        let result = SignatureDatabase::from_bytes(&data);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn from_bytes_rejects_list_size_exceeding_data() {
        // ARRANGE
        let mut data = vec![0u8; SIGNATURE_LIST_HEADER_SIZE];
        let bad_size = (SIGNATURE_LIST_HEADER_SIZE + 100) as u32;
        data[16..20].copy_from_slice(&bad_size.to_le_bytes());

        // ACT
        let result = SignatureDatabase::from_bytes(&data);

        // ASSERT
        assert!(result.is_err());
    }
}
