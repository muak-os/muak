//! Key type and hierarchy definitions.

use ring::rand::{SecureRandom as _, SystemRandom};
use x509_cert::Certificate;

use super::cert;
use super::rsa2048;
use crate::error::{Result, SboltError};

/// Type of Secure Boot key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyType {
    /// Platform Key.
    Pk,
    /// Key Exchange Key.
    Kek,
    /// Signature Database Key.
    Db,
}

impl KeyType {
    /// Returns the common name suffix for this key type.
    #[must_use]
    pub(crate) fn cn_suffix(self) -> &'static str {
        match self {
            Self::Pk => "Platform Key",
            Self::Kek => "Key Exchange Key",
            Self::Db => "Signature Database Key",
        }
    }

    /// Returns the filename prefix for this key type.
    #[must_use]
    pub(crate) fn file_prefix(self) -> &'static str {
        match self {
            Self::Pk => "pk",
            Self::Kek => "kek",
            Self::Db => "db",
        }
    }
}

/// A key pair consisting of a signer and its certificate.
pub struct KeyPair {
    /// RSA-2048 signer for this key.
    pub signer: rsa2048::Signer,
    /// X.509 certificate for this key.
    pub certificate: Certificate,
    /// Type of Secure Boot key.
    pub key_type: KeyType,
}

/// The complete Secure Boot key hierarchy.
pub struct Bundle {
    /// Platform Key pair.
    pub pk: KeyPair,
    /// Key Exchange Key pair.
    pub kek: KeyPair,
    /// Signature Database Key pair.
    pub db: KeyPair,
    /// Owner GUID for all keys.
    pub owner_guid: uefi::Guid,
}

impl Bundle {
    /// Generate a new key hierarchy with the given organization name.
    ///
    /// # Errors
    ///
    /// Returns an error if any key, certificate, or owner GUID generation step
    /// fails.
    pub fn generate(org_name: &str) -> Result<Self> {
        let (pk_signer, pk_cert) =
            cert::generate_pk(&format!("{org_name} {}", KeyType::Pk.cn_suffix()))?;

        let (kek_signer, kek_cert) = cert::generate_kek(
            &format!("{org_name} {}", KeyType::Kek.cn_suffix()),
            &pk_signer,
            &pk_cert,
        )?;

        let (db_signer, db_cert) =
            cert::generate_db(&format!("{org_name} Signing Key"), &kek_signer, &kek_cert)?;

        let owner_guid = Self::generate_owner_guid()?;

        Ok(Self {
            pk: KeyPair {
                signer: pk_signer,
                certificate: pk_cert,
                key_type: KeyType::Pk,
            },
            kek: KeyPair {
                signer: kek_signer,
                certificate: kek_cert,
                key_type: KeyType::Kek,
            },
            db: KeyPair {
                signer: db_signer,
                certificate: db_cert,
                key_type: KeyType::Db,
            },
            owner_guid,
        })
    }

    /// Generate a random owner GUID.
    fn generate_owner_guid() -> Result<uefi::Guid> {
        let rng = SystemRandom::new();
        let mut bytes = [0_u8; 16];
        rng.fill(&mut bytes)
            .map_err(|_guid_error| SboltError::KeyGeneration("failed to generate GUID".into()))?;

        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;

        Ok(uefi::Guid::from_bytes(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_type_helpers_return_expected_strings() {
        // ACT & ASSERT
        assert_eq!(KeyType::Pk.cn_suffix(), "Platform Key");
        assert_eq!(KeyType::Kek.cn_suffix(), "Key Exchange Key");
        assert_eq!(KeyType::Db.cn_suffix(), "Signature Database Key");
        assert_eq!(KeyType::Pk.file_prefix(), "pk");
        assert_eq!(KeyType::Kek.file_prefix(), "kek");
        assert_eq!(KeyType::Db.file_prefix(), "db");
    }

    #[test]
    fn generate_builds_complete_hierarchy() {
        // ARRANGE
        let org_name = "Muak Test";

        // ACT
        let hierarchy = Bundle::generate(org_name).expect("generate hierarchy");

        // ASSERT
        assert_eq!(hierarchy.pk.key_type, KeyType::Pk);
        assert_eq!(hierarchy.kek.key_type, KeyType::Kek);
        assert_eq!(hierarchy.db.key_type, KeyType::Db);
        assert_ne!(hierarchy.owner_guid, uefi::Guid::from_bytes([0_u8; 16]));
    }
}
