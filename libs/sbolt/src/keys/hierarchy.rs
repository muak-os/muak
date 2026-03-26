//! Key type and hierarchy definitions

use ring::rand::{SecureRandom, SystemRandom};
use x509_cert::Certificate;

use super::cert::{generate_db_certificate, generate_kek_certificate, generate_pk_certificate};
use super::signer::Rsa2048Signer;
use crate::Result;

/// Type of Secure Boot key
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyType {
    Pk,
    Kek,
    Db,
}

impl KeyType {
    /// Returns the common name suffix for this key type
    pub fn cn_suffix(&self) -> &'static str {
        match self {
            Self::Pk => "Platform Key",
            Self::Kek => "Key Exchange Key",
            Self::Db => "Signature Database Key",
        }
    }

    /// Returns the filename prefix for this key type
    pub fn file_prefix(&self) -> &'static str {
        match self {
            Self::Pk => "pk",
            Self::Kek => "kek",
            Self::Db => "db",
        }
    }
}

/// A key pair consisting of a signer and its certificate
pub struct KeyPair {
    pub signer: Rsa2048Signer,
    pub certificate: Certificate,
    pub key_type: KeyType,
}

/// The complete Secure Boot key hierarchy
pub struct KeyHierarchy {
    pub pk: KeyPair,
    pub kek: KeyPair,
    pub db: KeyPair,
    pub owner_guid: uefi::Guid,
}

impl KeyHierarchy {
    /// Generate a new key hierarchy with the given organization name
    pub fn generate(org_name: &str) -> Result<Self> {
        let (pk_signer, pk_cert) = generate_pk_certificate(&format!("{org_name} Platform Key"))?;

        let (kek_signer, kek_cert) = generate_kek_certificate(
            &format!("{org_name} Key Exchange Key"),
            &pk_signer,
            &pk_cert,
        )?;

        let (db_signer, db_cert) =
            generate_db_certificate(&format!("{org_name} Signing Key"), &kek_signer, &kek_cert)?;

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

    /// Generate a random owner GUID
    fn generate_owner_guid() -> Result<uefi::Guid> {
        let rng = SystemRandom::new();
        let mut bytes = [0u8; 16];
        rng.fill(&mut bytes)
            .map_err(|_| crate::Error::KeyGeneration("failed to generate GUID".into()))?;

        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;

        Ok(uefi::Guid::from_bytes(bytes))
    }
}
