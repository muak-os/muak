//! Key storage operations (PEM file read/write).

use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;

use der::pem::LineEnding;
use der::{Decode as _, Encode as _};
use x509_cert::Certificate;

use super::hierarchy::{Bundle, KeyPair, KeyType};
use super::rsa2048;
use crate::error::{Result, SboltError};

/// Save the key hierarchy to a directory.
///
/// # Errors
///
/// Returns an error if a directory or any key material file cannot be written.
pub fn save_hierarchy(hierarchy: &Bundle, dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;

    save_keypair(&hierarchy.pk, dir)?;
    save_keypair(&hierarchy.kek, dir)?;
    save_keypair(&hierarchy.db, dir)?;

    let guid_path = dir.join("owner.guid");
    write_private_file(&guid_path, format!("{}", hierarchy.owner_guid).as_bytes())?;

    Ok(())
}

/// Save a single key pair to a directory.
fn save_keypair(keypair: &KeyPair, dir: &Path) -> Result<()> {
    let prefix = keypair.key_type.file_prefix();

    let key_pem = pkcs8_to_pem(&keypair.signer.to_pkcs8_der()?)?;
    let key_path = dir.join(format!("{prefix}.key"));
    write_private_file(&key_path, key_pem.as_bytes())?;

    let cert_pem = cert_to_pem(&keypair.certificate)?;
    let cert_path = dir.join(format!("{prefix}.crt"));
    std::fs::write(&cert_path, cert_pem)?;

    let cert_der = keypair.certificate.to_der()?;
    let der_path = dir.join(format!("{prefix}.der"));
    std::fs::write(&der_path, cert_der)?;

    Ok(())
}

/// Load a key pair from key and certificate files.
///
/// # Errors
///
/// Returns an error if the key or certificate cannot be read or decoded.
pub fn load_pair(key_path: &Path, cert_path: &Path, key_type: KeyType) -> Result<KeyPair> {
    let key_pem = std::fs::read_to_string(key_path)?;
    let key_der = pem_to_pkcs8_der(&key_pem)?;
    let signer = rsa2048::Signer::from_pkcs8_der(&key_der)?;

    let cert_pem = std::fs::read_to_string(cert_path)?;
    let certificate = pem_to_cert(&cert_pem)?;

    Ok(KeyPair {
        signer,
        certificate,
        key_type,
    })
}

/// Load a key hierarchy from a directory.
///
/// # Errors
///
/// Returns an error if any stored key material or the owner GUID cannot be read
/// or decoded.
pub fn load_hierarchy(dir: &Path) -> Result<Bundle> {
    let pk = load_pair(&dir.join("pk.key"), &dir.join("pk.crt"), KeyType::Pk)?;
    let kek = load_pair(&dir.join("kek.key"), &dir.join("kek.crt"), KeyType::Kek)?;
    let db = load_pair(&dir.join("db.key"), &dir.join("db.crt"), KeyType::Db)?;

    let guid_str = std::fs::read_to_string(dir.join("owner.guid"))?;
    let owner_guid = uefi::Guid::try_parse(guid_str.trim()).map_err(|_guid_parse_error| {
        SboltError::KeyStorage(format!("invalid GUID format: {guid_str}"))
    })?;

    Ok(Bundle {
        pk,
        kek,
        db,
        owner_guid,
    })
}

/// Write a file with owner-read-write only permissions (0o600).
fn write_private_file(path: &Path, content: &[u8]) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(content)?;
    Ok(())
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
    Certificate::from_der(doc.as_bytes()).map_err(SboltError::Der)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::keys::hierarchy::Bundle;

    fn test_dir(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("sbolt-{name}-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create test dir");
        path
    }

    #[test]
    fn save_and_load_key_hierarchy_round_trip() -> Result<()> {
        // ARRANGE
        let hierarchy = Bundle::generate("Storage Test")?;
        let dir = test_dir("storage-roundtrip");

        // ACT
        save_hierarchy(&hierarchy, &dir)?;
        let loaded = load_hierarchy(&dir)?;

        // ASSERT
        assert_eq!(loaded.pk.key_type, KeyType::Pk);
        assert_eq!(loaded.kek.key_type, KeyType::Kek);
        assert_eq!(loaded.db.key_type, KeyType::Db);
        assert_eq!(loaded.owner_guid, hierarchy.owner_guid);

        Ok(())
    }

    #[test]
    fn save_key_hierarchy_writes_private_key_permissions() -> Result<()> {
        // ARRANGE
        let hierarchy = Bundle::generate("Storage Permissions")?;
        let dir = test_dir("storage-permissions");

        // ACT
        save_hierarchy(&hierarchy, &dir)?;
        let metadata = std::fs::metadata(dir.join("pk.key"))?;

        // ASSERT
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);

        Ok(())
    }

    #[test]
    fn load_key_hierarchy_rejects_invalid_guid() -> Result<()> {
        // ARRANGE
        let hierarchy = Bundle::generate("Storage Invalid Guid")?;
        let dir = test_dir("storage-invalid-guid");
        save_hierarchy(&hierarchy, &dir)?;
        std::fs::write(dir.join("owner.guid"), "not-a-guid")?;

        // ACT
        let result = load_hierarchy(&dir);

        // ASSERT
        assert!(result.is_err());

        Ok(())
    }
}
