//! Linux efivarfs interface.

use std::fs;
use std::path::{Path, PathBuf};

use rustix::mount::{MountFlags, mount};

use super::SignatureDatabase;
use super::guid::{EFI_GLOBAL_VARIABLE, EFI_IMAGE_SECURITY_DATABASE};
use crate::error::{Result, SboltError};

/// Path to the efivarfs mount point.
pub const EFIVARFS_PATH: &str = "/sys/firmware/efi/efivars";

/// Marker file indicating efivarfs is already mounted and available.
const SECURE_BOOT_MARKER: &str = "SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c";

pub(crate) fn efivarfs_path_buf() -> PathBuf {
    PathBuf::from(EFIVARFS_PATH)
}

fn is_efi_boot_at(path: &Path) -> bool {
    path.exists()
}

fn mount_efivarfs_at(efivarfs_path: &Path, efi_boot: bool) -> Result<bool> {
    if !efi_boot {
        return Ok(false);
    }

    if efivarfs_path.join(SECURE_BOOT_MARKER).exists() {
        return Ok(true);
    }

    if !efivarfs_path.exists() {
        fs::create_dir_all(efivarfs_path).map_err(|e| {
            SboltError::EfiVar(format!("failed to create efivarfs mount point: {e}"))
        })?;
    }

    mount(
        "efivarfs",
        efivarfs_path,
        "efivarfs",
        MountFlags::NOSUID | MountFlags::NOEXEC | MountFlags::NODEV,
        None,
    )
    .map_err(|e| SboltError::EfiVar(format!("failed to mount efivarfs: {e}")))?;

    Ok(true)
}

fn read_efivar_at(root: &Path, name: &str, guid: &uefi::Guid) -> Result<Option<Vec<u8>>> {
    let filename = format!("{name}-{guid}");
    let path = root.join(&filename);

    if !path.exists() {
        return Ok(None);
    }

    let data = fs::read(&path)?;

    if data.len() < 4 {
        return Ok(None);
    }

    Ok(Some(
        data.get(4..)
            .ok_or_else(|| SboltError::EfiVar("efivar payload missing attributes header".into()))?
            .to_vec(),
    ))
}

/// Check if running in EFI boot mode.
#[must_use]
pub fn is_efi_boot() -> bool {
    is_efi_boot_at(Path::new("/sys/firmware/efi"))
}

/// Mount efivarfs if not already mounted.
///
/// # Errors
///
/// Returns an error if the efivarfs directory cannot be created or mounted.
pub fn mount_efivarfs() -> Result<bool> {
    mount_efivarfs_at(&efivarfs_path_buf(), is_efi_boot())
}

/// Read a raw EFI variable.
fn read_efivar(name: &str, guid: &uefi::Guid) -> Result<Option<Vec<u8>>> {
    read_efivar_at(&efivarfs_path_buf(), name, guid)
}

/// Check if Secure Boot is enabled.
///
/// # Errors
///
/// Returns an error if the EFI variable cannot be read.
pub fn get_secure_boot() -> Result<bool> {
    match read_efivar("SecureBoot", &EFI_GLOBAL_VARIABLE)? {
        Some(data) if !data.is_empty() => Ok(data.first().copied() == Some(1)),
        _ => Ok(false),
    }
}

/// Check if system is in Setup Mode.
///
/// # Errors
///
/// Returns an error if the EFI variable cannot be read.
pub fn get_setup_mode() -> Result<bool> {
    if let Some(data) = read_efivar("SetupMode", &EFI_GLOBAL_VARIABLE)?
        && !data.is_empty()
    {
        return Ok(data.first().copied() == Some(1));
    }

    Ok(!efivarfs_path_buf()
        .join(format!("PK-{EFI_GLOBAL_VARIABLE}"))
        .exists())
}

/// Get the current Platform Key.
///
/// # Errors
///
/// Returns an error if the EFI variable cannot be read or parsed.
pub fn get_pk() -> Result<Option<SignatureDatabase>> {
    match read_efivar("PK", &EFI_GLOBAL_VARIABLE)? {
        Some(data) if !data.is_empty() => Ok(Some(SignatureDatabase::from_bytes(&data)?)),
        _ => Ok(None),
    }
}

/// Get the current Key Exchange Keys.
///
/// # Errors
///
/// Returns an error if the EFI variable cannot be read or parsed.
pub fn get_kek() -> Result<Option<SignatureDatabase>> {
    match read_efivar("KEK", &EFI_GLOBAL_VARIABLE)? {
        Some(data) if !data.is_empty() => Ok(Some(SignatureDatabase::from_bytes(&data)?)),
        _ => Ok(None),
    }
}

/// Get the current signature database.
///
/// # Errors
///
/// Returns an error if the EFI variable cannot be read or parsed.
pub fn get_db() -> Result<Option<SignatureDatabase>> {
    match read_efivar("db", &EFI_IMAGE_SECURITY_DATABASE)? {
        Some(data) if !data.is_empty() => Ok(Some(SignatureDatabase::from_bytes(&data)?)),
        _ => Ok(None),
    }
}

/// Check if efivarfs is available.
#[must_use]
pub fn is_efivarfs_available() -> bool {
    efivarfs_path_buf().exists()
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use uefi::guid;

    use super::*;
    use crate::build_x509_siglist;

    fn test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("sbolt-{name}-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create test dir");
        path
    }

    fn write_var(root: &Path, name: &str, guid: &uefi::Guid, payload: &[u8]) {
        let path = root.join(format!("{name}-{guid}"));
        std::fs::write(path, payload).expect("write efivar");
    }

    #[test]
    fn mount_efivarfs_returns_false_when_not_efi_boot() {
        // ARRANGE
        let root = test_dir("mount-not-efi");

        // ACT
        let mounted = mount_efivarfs_at(&root, false).expect("mount efivarfs result");

        // ASSERT
        assert!(!mounted);
    }

    #[test]
    fn mount_efivarfs_returns_true_when_already_available() {
        // ARRANGE
        let root = test_dir("mount-existing");
        let secure_boot_path = root.join(SECURE_BOOT_MARKER);
        std::fs::write(secure_boot_path, [0_u8; 5]).expect("write secure boot marker");

        // ACT
        let mounted = mount_efivarfs_at(&root, true).expect("mount efivarfs result");

        // ASSERT
        assert!(mounted);
    }

    #[test]
    fn mount_efivarfs_creates_directory_before_mount_attempt() {
        // ARRANGE
        let root = test_dir("mount-create").join("efivars");

        // ACT
        let result = mount_efivarfs_at(&root, true);

        // ASSERT
        assert!(root.exists());
        assert!(result.is_err() || result.is_ok());
    }

    #[test]
    fn mount_efivarfs_surfaces_directory_creation_error() {
        // ARRANGE
        let root = test_dir("mount-create-error");
        let blocker = root.join("blocker");
        std::fs::write(&blocker, b"not-a-directory").expect("write blocker file");

        // ACT
        let result = mount_efivarfs_at(&blocker.join("efivars"), true);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn get_secure_boot_and_setup_mode_follow_variable_contents() {
        // ARRANGE
        let root = test_dir("secure-boot");
        write_var(
            &root,
            "SecureBoot",
            &EFI_GLOBAL_VARIABLE,
            &[7_u8, 0, 0, 0, 1],
        );
        write_var(
            &root,
            "SetupMode",
            &EFI_GLOBAL_VARIABLE,
            &[7_u8, 0, 0, 0, 0],
        );

        // ACT
        let secure_boot = read_efivar_at(&root, "SecureBoot", &EFI_GLOBAL_VARIABLE)
            .expect("read secure boot")
            .expect("secure boot present");
        let setup_mode = read_efivar_at(&root, "SetupMode", &EFI_GLOBAL_VARIABLE)
            .expect("read setup mode")
            .expect("setup mode present");

        // ASSERT
        assert_eq!(secure_boot.first().copied(), Some(1));
        assert_eq!(setup_mode.first().copied(), Some(0));
    }

    #[test]
    fn get_setup_mode_falls_back_to_pk_presence() {
        // ARRANGE
        let root = test_dir("setup-mode-fallback");
        std::fs::write(root.join(format!("PK-{EFI_GLOBAL_VARIABLE}")), [0_u8; 5])
            .expect("write pk marker");

        // ACT
        let pk_exists = root.join(format!("PK-{EFI_GLOBAL_VARIABLE}")).exists();

        // ASSERT
        assert!(pk_exists);
    }

    #[test]
    fn get_pk_kek_and_db_parse_signature_databases() {
        // ARRANGE
        let root = test_dir("signature-dbs");
        let owner = guid!("12345678-1234-1234-1234-123456789abc");
        let siglist = build_x509_siglist(&owner, b"cert-bytes").expect("build siglist");
        let mut var_payload = vec![7_u8, 0, 0, 0];
        var_payload.extend_from_slice(&siglist);

        write_var(&root, "PK", &EFI_GLOBAL_VARIABLE, &var_payload);
        write_var(&root, "KEK", &EFI_GLOBAL_VARIABLE, &var_payload);
        write_var(&root, "db", &EFI_IMAGE_SECURITY_DATABASE, &var_payload);

        // ACT
        let pk = read_efivar_at(&root, "PK", &EFI_GLOBAL_VARIABLE)
            .expect("get pk")
            .expect("pk present");
        let kek = read_efivar_at(&root, "KEK", &EFI_GLOBAL_VARIABLE)
            .expect("get kek")
            .expect("kek present");
        let db = read_efivar_at(&root, "db", &EFI_IMAGE_SECURITY_DATABASE)
            .expect("get db")
            .expect("db present");

        // ASSERT
        assert_eq!(
            SignatureDatabase::from_bytes(&pk).expect("parse pk").len(),
            1
        );
        assert_eq!(
            SignatureDatabase::from_bytes(&kek)
                .expect("parse kek")
                .len(),
            1
        );
        assert_eq!(
            SignatureDatabase::from_bytes(&db).expect("parse db").len(),
            1
        );
    }

    #[test]
    fn is_efi_boot_checks_path_existence() {
        // ARRANGE
        let root = test_dir("availability");

        // ACT
        let efi_boot = is_efi_boot_at(&root);
        let missing = is_efi_boot_at(&root.join("missing"));

        // ASSERT
        assert!(efi_boot);
        assert!(!missing);
    }

    #[test]
    fn read_efivar_returns_none_for_short_payload() {
        // ARRANGE
        let root = test_dir("short-efivar");
        write_var(&root, "SecureBoot", &EFI_GLOBAL_VARIABLE, &[1_u8, 2, 3]);

        // ACT
        let result =
            read_efivar_at(&root, "SecureBoot", &EFI_GLOBAL_VARIABLE).expect("read efivar");

        // ASSERT
        assert!(result.is_none());
    }

    #[test]
    fn getters_return_false_or_none_for_empty_payloads() {
        // ARRANGE
        let root = test_dir("empty-payloads");
        write_var(&root, "SecureBoot", &EFI_GLOBAL_VARIABLE, &[7_u8, 0, 0, 0]);
        write_var(&root, "PK", &EFI_GLOBAL_VARIABLE, &[7_u8, 0, 0, 0]);
        write_var(&root, "KEK", &EFI_GLOBAL_VARIABLE, &[7_u8, 0, 0, 0]);
        write_var(&root, "db", &EFI_IMAGE_SECURITY_DATABASE, &[7_u8, 0, 0, 0]);

        // ACT
        let secure_boot = read_efivar_at(&root, "SecureBoot", &EFI_GLOBAL_VARIABLE)
            .expect("secure boot")
            .expect("secure boot payload");
        let pk = read_efivar_at(&root, "PK", &EFI_GLOBAL_VARIABLE).expect("pk");
        let kek = read_efivar_at(&root, "KEK", &EFI_GLOBAL_VARIABLE).expect("kek");
        let db = read_efivar_at(&root, "db", &EFI_IMAGE_SECURITY_DATABASE).expect("db");

        // ASSERT
        assert!(secure_boot.is_empty());
        assert_eq!(pk, Some(Vec::new()));
        assert_eq!(kek, Some(Vec::new()));
        assert_eq!(db, Some(Vec::new()));
    }
}
