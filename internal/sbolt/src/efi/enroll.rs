//! Key enrollment to UEFI firmware via efivarfs

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use der::Encode;
use rustix::fs::{Mode, OFlags, open};
use rustix::ioctl::{Getter, Setter, ioctl};
use uefi::runtime::VariableAttributes;

use super::authvar::sign_efi_variable;
use super::efivarfs::{EFIVARFS_PATH, get_setup_mode, is_efivarfs_available};
use super::guid::{EFI_GLOBAL_VARIABLE, EFI_IMAGE_SECURITY_DATABASE};
use super::siglist::SignatureDatabase;

use crate::keys::KeyHierarchy;
use crate::{Error, Result};

/// FS_IOC_GETFLAGS opcode (read direction, 'f' group, seq 1, c_long size)
const FS_IOC_GETFLAGS: u32 = rustix::ioctl::opcode::read::<std::ffi::c_long>(b'f', 1);

/// FS_IOC_SETFLAGS opcode (write direction, 'f' group, seq 2, c_long size)
const FS_IOC_SETFLAGS: u32 = rustix::ioctl::opcode::write::<std::ffi::c_long>(b'f', 2);

/// Immutable file attribute flag
const FS_IMMUTABLE_FL: std::ffi::c_long = 0x00000010;

/// Clear the immutable flag on an efivarfs file
fn unset_immutable(path: &Path) -> Result<()> {
    let file = open(path, OFlags::RDWR, Mode::empty()).map_err(|e| {
        Error::EfiVar(format!(
            "failed to open efivarfs file: {}",
            std::io::Error::from(e)
        ))
    })?;

    // SAFETY: FS_IOC_GETFLAGS is valid for this opcode and expects a c_long output
    let flags = unsafe { ioctl(&file, Getter::<FS_IOC_GETFLAGS, std::ffi::c_long>::new()) }
        .map_err(|e| {
            Error::EfiVar(format!(
                "ioctl GETFLAGS failed: {}",
                std::io::Error::from(e)
            ))
        })?;

    if flags & FS_IMMUTABLE_FL != 0 {
        // SAFETY: FS_IOC_SETFLAGS is valid for this opcode and expects a c_long input
        unsafe {
            ioctl(
                &file,
                Setter::<FS_IOC_SETFLAGS, std::ffi::c_long>::new(flags & !FS_IMMUTABLE_FL),
            )
        }
        .map_err(|e| {
            Error::EfiVar(format!(
                "ioctl SETFLAGS failed: {}",
                std::io::Error::from(e)
            ))
        })?;
    }

    Ok(())
}

/// Write an authenticated EFI variable to efivarfs
fn write_efivar_auth(
    name: &str,
    vendor_guid: &uefi::Guid,
    content: &[u8],
    hierarchy: &KeyHierarchy,
    use_pk_signer: bool,
) -> Result<()> {
    let filename = format!("{}-{}", name, vendor_guid);
    let path = Path::new(EFIVARFS_PATH).join(&filename);

    // Select the appropriate signer
    let (signer, certificate) = if use_pk_signer {
        (&hierarchy.pk.signer, &hierarchy.pk.certificate)
    } else {
        (&hierarchy.kek.signer, &hierarchy.kek.certificate)
    };

    // Sign the variable update
    let payload = sign_efi_variable(
        name,
        vendor_guid,
        VariableAttributes::NON_VOLATILE
            .union(VariableAttributes::BOOTSERVICE_ACCESS)
            .union(VariableAttributes::RUNTIME_ACCESS)
            .union(VariableAttributes::TIME_BASED_AUTHENTICATED_WRITE_ACCESS),
        content,
        signer,
        certificate,
    )?;

    // If the file exists, clear immutable flag first
    if path.exists() {
        unset_immutable(&path)?;
    }

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)?;

    file.write_all(&payload)?;
    file.flush()?;

    Ok(())
}

/// Enroll a signature database to the db variable
fn enroll_db(sigdb: &SignatureDatabase, hierarchy: &KeyHierarchy) -> Result<()> {
    write_efivar_auth(
        "db",
        &EFI_IMAGE_SECURITY_DATABASE,
        &sigdb.to_bytes(),
        hierarchy,
        false,
    )
}

/// Enroll a signature database to the KEK variable
fn enroll_kek(sigdb: &SignatureDatabase, hierarchy: &KeyHierarchy) -> Result<()> {
    write_efivar_auth(
        "KEK",
        &EFI_GLOBAL_VARIABLE,
        &sigdb.to_bytes(),
        hierarchy,
        true,
    )
}

/// Enroll a signature database to the PK variable
fn enroll_pk(sigdb: &SignatureDatabase, hierarchy: &KeyHierarchy) -> Result<()> {
    write_efivar_auth(
        "PK",
        &EFI_GLOBAL_VARIABLE,
        &sigdb.to_bytes(),
        hierarchy,
        true,
    )
}

/// Enroll the complete key hierarchy to UEFI firmware
pub fn enroll_keys(hierarchy: &KeyHierarchy) -> Result<()> {
    if !is_efivarfs_available() {
        return Err(Error::EfiVar("efivarfs not available".into()));
    }

    let setup_mode = get_setup_mode()?;

    if !setup_mode {
        return Err(Error::EfiVar(
            "system is not in Setup Mode, cannot enroll keys".into(),
        ));
    }

    let mut db_sigdb = SignatureDatabase::new();
    let db_cert_der = hierarchy
        .db
        .certificate
        .to_der()
        .map_err(|e| Error::EfiVar(format!("encode db cert: {e}")))?;

    db_sigdb.add_x509(&hierarchy.owner_guid, &db_cert_der);

    let mut kek_sigdb = SignatureDatabase::new();
    kek_sigdb.add_x509(
        &hierarchy.owner_guid,
        &hierarchy
            .kek
            .certificate
            .to_der()
            .map_err(|e| Error::EfiVar(format!("encode kek cert: {e}")))?,
    );

    let mut pk_sigdb = SignatureDatabase::new();
    pk_sigdb.add_x509(
        &hierarchy.owner_guid,
        &hierarchy
            .pk
            .certificate
            .to_der()
            .map_err(|e| Error::EfiVar(format!("encode pk cert: {e}")))?,
    );

    enroll_db(&db_sigdb, hierarchy)
        .map_err(|e| Error::EfiVar(format!("failed to enroll db: {e}")))?;

    enroll_kek(&kek_sigdb, hierarchy)
        .map_err(|e| Error::EfiVar(format!("failed to enroll KEK: {e}")))?;

    enroll_pk(&pk_sigdb, hierarchy)
        .map_err(|e| Error::EfiVar(format!("failed to enroll PK: {e}")))?;

    Ok(())
}
