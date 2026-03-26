//! Key storage operations (PEM file read/write)

use std::path::Path;

use der::{Decode, Encode, pem::LineEnding};
use x509_cert::Certificate;

use super::hierarchy::{KeyHierarchy, KeyPair, KeyType};
use super::signer::Rsa2048Signer;
use crate::{Error, Result};

/// Save the key hierarchy to a directory
pub fn save_key_hierarchy(hierarchy: &KeyHierarchy, dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;

    save_keypair(&hierarchy.pk, dir)?;
    save_keypair(&hierarchy.kek, dir)?;
    save_keypair(&hierarchy.db, dir)?;

    let guid_path = dir.join("owner.guid");
    std::fs::write(&guid_path, format!("{}", hierarchy.owner_guid))?;

    Ok(())
}

/// Save a single key pair to a directory
fn save_keypair(keypair: &KeyPair, dir: &Path) -> Result<()> {
    let prefix = keypair.key_type.file_prefix();

    let key_pem = pkcs8_to_pem(&keypair.signer.to_pkcs8_der()?)?;
    let key_path = dir.join(format!("{prefix}.key"));
    std::fs::write(&key_path, key_pem)?;

    let cert_pem = cert_to_pem(&keypair.certificate)?;
    let cert_path = dir.join(format!("{prefix}.crt"));
    std::fs::write(&cert_path, cert_pem)?;

    let cert_der = keypair.certificate.to_der()?;
    let der_path = dir.join(format!("{prefix}.der"));
    std::fs::write(&der_path, cert_der)?;

    Ok(())
}

/// Load a key pair from key and certificate files
pub fn load_keypair(key_path: &Path, cert_path: &Path, key_type: KeyType) -> Result<KeyPair> {
    let key_pem = std::fs::read_to_string(key_path)?;
    let key_der = pem_to_pkcs8_der(&key_pem)?;
    let signer = Rsa2048Signer::from_pkcs8_der(&key_der)?;

    let cert_pem = std::fs::read_to_string(cert_path)?;
    let certificate = pem_to_cert(&cert_pem)?;

    Ok(KeyPair {
        signer,
        certificate,
        key_type,
    })
}

/// Load a key hierarchy from a directory
pub fn load_key_hierarchy(dir: &Path) -> Result<KeyHierarchy> {
    let pk = load_keypair(&dir.join("pk.key"), &dir.join("pk.crt"), KeyType::Pk)?;
    let kek = load_keypair(&dir.join("kek.key"), &dir.join("kek.crt"), KeyType::Kek)?;
    let db = load_keypair(&dir.join("db.key"), &dir.join("db.crt"), KeyType::Db)?;

    let guid_str = std::fs::read_to_string(dir.join("owner.guid"))?;
    let owner_guid = uefi::Guid::try_parse(&guid_str.trim())
        .map_err(|_| Error::KeyStorage(format!("invalid GUID format: {}", &guid_str)))?;

    Ok(KeyHierarchy {
        pk,
        kek,
        db,
        owner_guid,
    })
}

/// Convert PKCS#8 DER to PEM format.
fn pkcs8_to_pem(der: &[u8]) -> Result<String> {
    let doc = der::SecretDocument::try_from(der)?;
    let pem = doc.to_pem("PRIVATE KEY", LineEnding::LF)?;
    Ok(pem.to_string())
}

/// Convert PEM-encoded PKCS#8 to DER.
fn pem_to_pkcs8_der(pem: &str) -> Result<Vec<u8>> {
    let (_label, doc) = der::SecretDocument::from_pem(pem)?;
    Ok(doc.as_bytes().to_vec())
}

/// Convert certificate to PEM format.
fn cert_to_pem(cert: &Certificate) -> Result<String> {
    let der = cert.to_der()?;
    let doc = der::Document::try_from(der)?;
    let pem = doc.to_pem("CERTIFICATE", LineEnding::LF)?;
    Ok(pem)
}

/// Convert PEM-encoded certificate to Certificate.
fn pem_to_cert(pem: &str) -> Result<Certificate> {
    let (_label, doc) = der::Document::from_pem(pem)?;
    Certificate::from_der(doc.as_bytes()).map_err(Error::Der)
}
