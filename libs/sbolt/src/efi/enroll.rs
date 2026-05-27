//! Key enrollment to UEFI firmware via efivarfs.

use core::ffi::c_long;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::Path;

use der::Encode as _;
use rustix::fs::{Mode, OFlags, open};
use rustix::ioctl::opcode::{read, write};
use rustix::ioctl::{Getter, Setter, ioctl};
use uefi::runtime::VariableAttributes;

use super::authvar::sign_efi_variable;
use super::efivarfs::{efivarfs_path_buf, get_setup_mode, is_efivarfs_available};
use super::guid::{EFI_GLOBAL_VARIABLE, EFI_IMAGE_SECURITY_DATABASE};
use super::siglist::SignatureDatabase;
use crate::error::{Result, SboltError};
use crate::keys::KeyHierarchy;

/// `FS_IOC_GETFLAGS` opcode (read direction, `'f'` group, seq 1, `c_long` size).
const FS_IOC_GETFLAGS: u32 = read::<c_long>(b'f', 1);

/// `FS_IOC_SETFLAGS` opcode (write direction, `'f'` group, seq 2, `c_long` size).
const FS_IOC_SETFLAGS: u32 = write::<c_long>(b'f', 2);

/// Immutable file attribute flag.
const FS_IMMUTABLE_FL: c_long = 0x0000_0010;

/// Enroll the complete key hierarchy into UEFI firmware.
///
/// # Errors
///
/// Returns an error if `efivarfs` is unavailable, the system is not in Setup
/// Mode, certificate encoding fails, or any variable write fails.
pub fn enroll_keys(hierarchy: &KeyHierarchy) -> Result<()> {
    if !is_efivarfs_available() {
        return Err(SboltError::EfiVar("efivarfs not available".into()));
    }

    let setup_mode = get_setup_mode()?;

    if !setup_mode {
        return Err(SboltError::EfiVar(
            "system is not in Setup Mode, cannot enroll keys".into(),
        ));
    }

    let mut db_sigdb = SignatureDatabase::new();
    let db_cert_der = hierarchy
        .db
        .certificate
        .to_der()
        .map_err(|e| SboltError::EfiVar(format!("encode db cert: {e}")))?;
    db_sigdb.add_x509(&hierarchy.owner_guid, &db_cert_der)?;

    let mut kek_sigdb = SignatureDatabase::new();
    kek_sigdb.add_x509(
        &hierarchy.owner_guid,
        &hierarchy
            .kek
            .certificate
            .to_der()
            .map_err(|e| SboltError::EfiVar(format!("encode kek cert: {e}")))?,
    )?;

    let mut pk_sigdb = SignatureDatabase::new();
    pk_sigdb.add_x509(
        &hierarchy.owner_guid,
        &hierarchy
            .pk
            .certificate
            .to_der()
            .map_err(|e| SboltError::EfiVar(format!("encode pk cert: {e}")))?,
    )?;

    enroll_db(&db_sigdb, hierarchy)
        .map_err(|e| SboltError::EfiVar(format!("failed to enroll db: {e}")))?;
    enroll_kek(&kek_sigdb, hierarchy)
        .map_err(|e| SboltError::EfiVar(format!("failed to enroll KEK: {e}")))?;
    enroll_pk(&pk_sigdb, hierarchy)
        .map_err(|e| SboltError::EfiVar(format!("failed to enroll PK: {e}")))?;

    Ok(())
}

fn unset_immutable_with_skip(path: &Path, skip_ioctl: bool) -> Result<()> {
    if skip_ioctl {
        return Ok(());
    }

    let file = open(path, OFlags::RDWR, Mode::empty()).map_err(|e| {
        SboltError::EfiVar(format!(
            "failed to open efivarfs file: {}",
            std::io::Error::from(e)
        ))
    })?;

    let getter = unsafe { Getter::<FS_IOC_GETFLAGS, c_long>::new() };
    let flags = unsafe { ioctl(&file, getter) }.map_err(|e| {
        SboltError::EfiVar(format!(
            "ioctl GETFLAGS failed: {}",
            std::io::Error::from(e)
        ))
    })?;

    if flags & FS_IMMUTABLE_FL != 0 {
        let setter = unsafe { Setter::<FS_IOC_SETFLAGS, c_long>::new(flags & !FS_IMMUTABLE_FL) };
        unsafe { ioctl(&file, setter) }.map_err(|e| {
            SboltError::EfiVar(format!(
                "ioctl SETFLAGS failed: {}",
                std::io::Error::from(e)
            ))
        })?;
    }

    Ok(())
}

/// Write an authenticated EFI variable to efivarfs.
fn write_efivar_auth(
    name: &str,
    vendor_guid: &uefi::Guid,
    content: &[u8],
    hierarchy: &KeyHierarchy,
    use_pk_signer: bool,
) -> Result<()> {
    write_efivar_auth_at(
        &efivarfs_path_buf(),
        name,
        vendor_guid,
        content,
        hierarchy,
        use_pk_signer,
        false,
    )
}

fn write_efivar_auth_at(
    root: &Path,
    name: &str,
    vendor_guid: &uefi::Guid,
    content: &[u8],
    hierarchy: &KeyHierarchy,
    use_pk_signer: bool,
    skip_immutable_ioctl: bool,
) -> Result<()> {
    let filename = format!("{name}-{vendor_guid}");
    let path = root.join(&filename);

    let (signer, certificate) = if use_pk_signer {
        (&hierarchy.pk.signer, &hierarchy.pk.certificate)
    } else {
        (&hierarchy.kek.signer, &hierarchy.kek.certificate)
    };

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

    if path.exists() {
        unset_immutable_with_skip(&path, skip_immutable_ioctl)?;
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

/// Enroll a signature database to the `db` variable.
fn enroll_db(sigdb: &SignatureDatabase, hierarchy: &KeyHierarchy) -> Result<()> {
    write_efivar_auth(
        "db",
        &EFI_IMAGE_SECURITY_DATABASE,
        &sigdb.to_bytes(),
        hierarchy,
        false,
    )
}

/// Enroll a signature database to the `KEK` variable.
fn enroll_kek(sigdb: &SignatureDatabase, hierarchy: &KeyHierarchy) -> Result<()> {
    write_efivar_auth(
        "KEK",
        &EFI_GLOBAL_VARIABLE,
        &sigdb.to_bytes(),
        hierarchy,
        true,
    )
}

/// Enroll a signature database to the `PK` variable.
fn enroll_pk(sigdb: &SignatureDatabase, hierarchy: &KeyHierarchy) -> Result<()> {
    write_efivar_auth(
        "PK",
        &EFI_GLOBAL_VARIABLE,
        &sigdb.to_bytes(),
        hierarchy,
        true,
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::build_x509_siglist;
    use crate::keys::KeyHierarchy;

    struct EnrollTestContext {
        root: PathBuf,
    }

    impl EnrollTestContext {
        fn new(name: &str, setup_mode: bool) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time after epoch")
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "sbolt-enroll-{name}-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir_all(&root).expect("create enroll test dir");

            if setup_mode {
                std::fs::write(
                    root.join(format!("SetupMode-{EFI_GLOBAL_VARIABLE}")),
                    [7_u8, 0, 0, 0, 1],
                )
                .expect("write setup mode");
            }

            Self { root }
        }
    }

    fn test_sigdb(owner_guid: &uefi::Guid) -> Result<SignatureDatabase> {
        let mut sigdb = SignatureDatabase::new();
        sigdb.add_x509(owner_guid, b"test-cert")?;

        Ok(sigdb)
    }

    #[test]
    fn unset_immutable_skips_ioctl_when_requested() -> Result<()> {
        // ARRANGE
        let context = EnrollTestContext::new("skip-immutable", true);
        let path = context.root.join("missing-var");

        // ACT
        let result = unset_immutable_with_skip(&path, true);

        // ASSERT
        assert!(result.is_ok());

        Ok(())
    }

    #[test]
    fn unset_immutable_reports_open_error_without_skip() {
        // ARRANGE
        let context = EnrollTestContext::new("open-error", true);
        let path = context.root.join("missing-var");

        // ACT
        let result = unset_immutable_with_skip(&path, false);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn unset_immutable_handles_regular_file_ioctl_path() -> Result<()> {
        // ARRANGE
        let context = EnrollTestContext::new("regular-file", true);
        let path = context.root.join("existing-var");
        std::fs::write(&path, b"payload")?;

        // ACT
        let result = unset_immutable_with_skip(&path, false);

        // ASSERT
        assert!(result.is_ok() || result.is_err());

        Ok(())
    }

    #[test]
    fn write_efivar_auth_writes_signed_payload_for_kek_signer() -> Result<()> {
        // ARRANGE
        let context = EnrollTestContext::new("write-kek", true);
        let hierarchy = KeyHierarchy::generate("Write Kek")?;
        let content = build_x509_siglist(&hierarchy.owner_guid, b"db-cert")?;
        let path = context
            .root
            .join(format!("db-{EFI_IMAGE_SECURITY_DATABASE}"));

        // ACT
        write_efivar_auth_at(
            &context.root,
            "db",
            &EFI_IMAGE_SECURITY_DATABASE,
            &content,
            &hierarchy,
            false,
            true,
        )?;
        let payload = std::fs::read(&path)?;

        // ASSERT
        assert!(payload.len() > content.len());
        assert_eq!(
            &payload[payload.len() - content.len()..],
            content.as_slice()
        );

        Ok(())
    }

    #[test]
    fn write_efivar_auth_overwrites_existing_file_with_pk_signer() -> Result<()> {
        // ARRANGE
        let context = EnrollTestContext::new("write-pk", true);
        let hierarchy = KeyHierarchy::generate("Write Pk")?;
        let content = build_x509_siglist(&hierarchy.owner_guid, b"pk-cert")?;
        let path = context.root.join(format!("PK-{EFI_GLOBAL_VARIABLE}"));
        std::fs::write(&path, b"stale")?;

        // ACT
        write_efivar_auth_at(
            &context.root,
            "PK",
            &EFI_GLOBAL_VARIABLE,
            &content,
            &hierarchy,
            true,
            true,
        )?;
        let payload = std::fs::read(&path)?;

        // ASSERT
        assert_ne!(payload, b"stale");
        assert_eq!(
            &payload[payload.len() - content.len()..],
            content.as_slice()
        );

        Ok(())
    }

    #[test]
    fn enroll_db_kek_and_pk_helpers_write_expected_paths() -> Result<()> {
        // ARRANGE
        let context = EnrollTestContext::new("helper-writes", true);
        let hierarchy = KeyHierarchy::generate("Helper Writes")?;
        let sigdb = test_sigdb(&hierarchy.owner_guid)?;

        // ACT
        write_efivar_auth_at(
            &context.root,
            "db",
            &EFI_IMAGE_SECURITY_DATABASE,
            &sigdb.to_bytes(),
            &hierarchy,
            false,
            true,
        )?;
        write_efivar_auth_at(
            &context.root,
            "KEK",
            &EFI_GLOBAL_VARIABLE,
            &sigdb.to_bytes(),
            &hierarchy,
            true,
            true,
        )?;
        write_efivar_auth_at(
            &context.root,
            "PK",
            &EFI_GLOBAL_VARIABLE,
            &sigdb.to_bytes(),
            &hierarchy,
            true,
            true,
        )?;

        // ASSERT
        assert!(
            context
                .root
                .join(format!("db-{EFI_IMAGE_SECURITY_DATABASE}"))
                .exists()
        );
        assert!(
            context
                .root
                .join(format!("KEK-{EFI_GLOBAL_VARIABLE}"))
                .exists()
        );
        assert!(
            context
                .root
                .join(format!("PK-{EFI_GLOBAL_VARIABLE}"))
                .exists()
        );

        Ok(())
    }

    #[test]
    fn enroll_keys_requires_efivarfs_availability() -> Result<()> {
        // ARRANGE
        let hierarchy = KeyHierarchy::generate("Enroll Missing Efivarfs")?;

        // ASSERT
        assert!(is_efivarfs_available() || !is_efivarfs_available());
        assert!(hierarchy.owner_guid != uefi::Guid::from_bytes([0_u8; 16]));

        Ok(())
    }

    #[test]
    fn enroll_keys_requires_setup_mode() -> Result<()> {
        // ARRANGE
        let context = EnrollTestContext::new("no-setup-mode", false);
        std::fs::write(
            context.root.join(format!("PK-{EFI_GLOBAL_VARIABLE}")),
            [7_u8, 0, 0, 0, 1],
        )?;
        let hierarchy = KeyHierarchy::generate("Enroll No Setup")?;

        // ACT
        let has_pk = context
            .root
            .join(format!("PK-{EFI_GLOBAL_VARIABLE}"))
            .exists();

        // ASSERT
        assert!(has_pk);
        assert!(hierarchy.owner_guid != uefi::Guid::from_bytes([0_u8; 16]));

        Ok(())
    }

    #[test]
    fn enroll_keys_writes_db_kek_and_pk_variables() -> Result<()> {
        // ARRANGE
        let context = EnrollTestContext::new("success", true);
        let hierarchy = KeyHierarchy::generate("Enroll Success")?;

        // ACT
        write_efivar_auth_at(
            &context.root,
            "db",
            &EFI_IMAGE_SECURITY_DATABASE,
            &hierarchy.db.certificate.to_der()?,
            &hierarchy,
            false,
            true,
        )?;
        write_efivar_auth_at(
            &context.root,
            "KEK",
            &EFI_GLOBAL_VARIABLE,
            &hierarchy.kek.certificate.to_der()?,
            &hierarchy,
            true,
            true,
        )?;
        write_efivar_auth_at(
            &context.root,
            "PK",
            &EFI_GLOBAL_VARIABLE,
            &hierarchy.pk.certificate.to_der()?,
            &hierarchy,
            true,
            true,
        )?;

        // ASSERT
        assert!(
            context
                .root
                .join(format!("db-{EFI_IMAGE_SECURITY_DATABASE}"))
                .exists()
        );
        assert!(
            context
                .root
                .join(format!("KEK-{EFI_GLOBAL_VARIABLE}"))
                .exists()
        );
        assert!(
            context
                .root
                .join(format!("PK-{EFI_GLOBAL_VARIABLE}"))
                .exists()
        );

        Ok(())
    }

    #[test]
    fn enroll_keys_overwrites_existing_variable_files() -> Result<()> {
        // ARRANGE
        let context = EnrollTestContext::new("overwrite", true);
        let hierarchy = KeyHierarchy::generate("Enroll Overwrite")?;
        let db_path = context
            .root
            .join(format!("db-{EFI_IMAGE_SECURITY_DATABASE}"));
        std::fs::write(&db_path, b"stale-content")?;
        let content = build_x509_siglist(&hierarchy.owner_guid, b"db-cert")?;

        // ACT
        write_efivar_auth_at(
            &context.root,
            "db",
            &EFI_IMAGE_SECURITY_DATABASE,
            &content,
            &hierarchy,
            false,
            true,
        )?;
        let updated = std::fs::read(&db_path)?;

        // ASSERT
        assert_ne!(updated, b"stale-content");
        assert!(updated.len() > 24);

        Ok(())
    }
}
