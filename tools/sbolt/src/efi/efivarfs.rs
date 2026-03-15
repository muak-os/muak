//! Linux efivarfs interface

use std::fs;
use std::path::Path;

use rustix::mount::{MountFlags, mount};

use super::SignatureDatabase;
use super::guid::{EFI_GLOBAL_VARIABLE, EFI_IMAGE_SECURITY_DATABASE};
use crate::{Error, Result};

pub const EFIVARFS_PATH: &str = "/sys/firmware/efi/efivars";

/// Check if running in EFI boot mode
pub fn is_efi_boot() -> bool {
    Path::new("/sys/firmware/efi").exists()
}

/// Mount efivarfs if not already mounted
pub fn mount_efivarfs() -> Result<bool> {
    if !is_efi_boot() {
        return Ok(false);
    }

    let test_path =
        Path::new(EFIVARFS_PATH).join("SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c");
    if test_path.exists() {
        return Ok(true);
    }

    let efivarfs_dir = Path::new(EFIVARFS_PATH);
    if !efivarfs_dir.exists() {
        fs::create_dir_all(efivarfs_dir)
            .map_err(|e| Error::EfiVar(format!("failed to create efivarfs mount point: {e}")))?;
    }

    mount(
        "efivarfs",
        EFIVARFS_PATH,
        "efivarfs",
        MountFlags::NOSUID | MountFlags::NOEXEC | MountFlags::NODEV,
        None,
    )
    .map_err(|e| Error::EfiVar(format!("failed to mount efivarfs: {e}")))?;

    Ok(true)
}

/// Read a raw EFI variable
fn read_efivar(name: &str, guid: &uefi::Guid) -> Result<Option<Vec<u8>>> {
    let filename = format!("{}-{}", name, guid);
    let path = Path::new(EFIVARFS_PATH).join(&filename);

    if !path.exists() {
        return Ok(None);
    }

    let data = fs::read(&path)?;

    if data.len() < 4 {
        return Ok(None);
    }

    Ok(Some(data[4..].to_vec()))
}

/// Check if Secure Boot is enabled
pub fn get_secure_boot() -> Result<bool> {
    match read_efivar("SecureBoot", &EFI_GLOBAL_VARIABLE)? {
        Some(data) if !data.is_empty() => Ok(data[0] == 1),
        _ => Ok(false),
    }
}

/// Check if system is in Setup Mode
pub fn get_setup_mode() -> Result<bool> {
    if let Some(data) = read_efivar("SetupMode", &EFI_GLOBAL_VARIABLE)?
        && !data.is_empty()
    {
        return Ok(data[0] == 1);
    }

    let pk_path = Path::new(EFIVARFS_PATH).join(format!("PK-{}", EFI_GLOBAL_VARIABLE));
    Ok(!pk_path.exists())
}

/// Get the current Platform Key
pub fn get_pk() -> Result<Option<SignatureDatabase>> {
    match read_efivar("PK", &EFI_GLOBAL_VARIABLE)? {
        Some(data) if !data.is_empty() => Ok(Some(SignatureDatabase::from_bytes(&data)?)),
        _ => Ok(None),
    }
}

/// Get the current Key Exchange Keys
pub fn get_kek() -> Result<Option<SignatureDatabase>> {
    match read_efivar("KEK", &EFI_GLOBAL_VARIABLE)? {
        Some(data) if !data.is_empty() => Ok(Some(SignatureDatabase::from_bytes(&data)?)),
        _ => Ok(None),
    }
}

/// Get the current Signature Database
pub fn get_db() -> Result<Option<SignatureDatabase>> {
    match read_efivar("db", &EFI_IMAGE_SECURITY_DATABASE)? {
        Some(data) if !data.is_empty() => Ok(Some(SignatureDatabase::from_bytes(&data)?)),
        _ => Ok(None),
    }
}

/// Check if efivarfs is available
pub fn is_efivarfs_available() -> bool {
    Path::new(EFIVARFS_PATH).exists()
}
