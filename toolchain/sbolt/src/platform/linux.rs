//! Linux `efivarfs` firmware variable backend.

use core::ffi::c_long;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use rustix::fs::{Mode, OFlags, open};
use rustix::ioctl::opcode::{read, write};
use rustix::ioctl::{Getter, Setter, ioctl};
use rustix::mount::{MountFlags, mount};

use crate::efi::variables::{Backend, Id, Update};
use crate::error::{Result, SboltError};

/// Linux EFI firmware directory path.
const EFI_FIRMWARE_PATH: &str = "/sys/firmware/efi";

/// Linux `efivarfs` mount point path.
const EFIVARFS_PATH: &str = "/sys/firmware/efi/efivars";

/// Linux `efivarfs` filesystem type name.
const EFIVARFS_TYPE: &str = "efivarfs";

/// Marker file indicating `efivarfs` is mounted and usable.
const SECURE_BOOT_MARKER: &str = "SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c";

/// Size of the Linux `efivarfs` attributes header.
const EFIVARFS_ATTRIBUTES_SIZE: usize = 4;

/// `FS_IOC_GETFLAGS` opcode for Linux inode flags.
const FS_IOC_GETFLAGS: u32 = read::<c_long>(b'f', 1);

/// `FS_IOC_SETFLAGS` opcode for Linux inode flags.
const FS_IOC_SETFLAGS: u32 = write::<c_long>(b'f', 2);

/// Linux immutable inode flag.
const FS_IMMUTABLE_FL: c_long = 0x0000_0010;

/// Firmware variable backend using Linux `efivarfs`.
#[derive(Debug, Clone)]
pub(crate) struct Efivarfs {
    efi_firmware_path: PathBuf,
    efivarfs_path: PathBuf,
}

impl Efivarfs {
    /// Create the default Linux firmware variable backend.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            efi_firmware_path: PathBuf::from(EFI_FIRMWARE_PATH),
            efivarfs_path: PathBuf::from(EFIVARFS_PATH),
        }
    }

    /// Return the filesystem path for a firmware variable.
    fn variable_path(&self, id: &Id) -> PathBuf {
        self.efivarfs_path.join(variable_filename(id))
    }

    /// Mount `efivarfs` if the system is EFI-booted and not already mounted.
    fn mount_efivarfs(&self) -> Result<bool> {
        if !self.is_firmware_boot() {
            return Ok(false);
        }

        if self.efivarfs_path.join(SECURE_BOOT_MARKER).exists() {
            return Ok(true);
        }

        if !self.efivarfs_path.exists() {
            create_efivarfs_dir(&self.efivarfs_path)?;
        }

        mount(
            EFIVARFS_TYPE,
            &self.efivarfs_path,
            EFIVARFS_TYPE,
            MountFlags::NOSUID | MountFlags::NOEXEC | MountFlags::NODEV,
            None,
        )
        .map_err(|e| SboltError::EfiVar(format!("failed to mount efivarfs: {e}")))?;

        Ok(true)
    }
}

impl Default for Efivarfs {
    /// Create the default Linux firmware variable backend.
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for Efivarfs {
    /// Return whether the system was booted through EFI firmware.
    fn is_firmware_boot(&self) -> bool {
        self.efi_firmware_path.exists()
    }

    /// Return whether the `efivarfs` mount point currently exists.
    fn is_available(&self) -> bool {
        self.efivarfs_path.exists()
    }

    /// Mount `efivarfs` when needed and report whether it is ready.
    fn ensure_ready(&self) -> Result<bool> {
        self.mount_efivarfs()
    }

    /// Return whether the `efivarfs` variable file exists.
    fn variable_exists(&self, id: &Id) -> bool {
        self.variable_path(id).exists()
    }

    /// Read an `efivarfs` payload without the Linux attributes header.
    fn read_variable(&self, id: &Id) -> Result<Option<Vec<u8>>> {
        let path = self.variable_path(id);

        if !path.exists() {
            return Ok(None);
        }

        let data = fs::read(path)?;

        if data.len() < EFIVARFS_ATTRIBUTES_SIZE {
            return Ok(None);
        }

        let payload = data
            .get(EFIVARFS_ATTRIBUTES_SIZE..)
            .ok_or_else(|| SboltError::EfiVar("efivar payload missing attributes header".into()))?;

        Ok(Some(payload.to_vec()))
    }

    /// Write an authenticated payload to an `efivarfs` variable file.
    fn write_variable(&self, update: Update<'_>) -> Result<()> {
        let path = self.variable_path(update.id());

        if path.exists() {
            unset_immutable(&path)?;
        }

        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        file.write_all(update.payload())?;
        file.flush()?;

        Ok(())
    }
}

/// Clear the immutable inode flag for an existing `efivarfs` file.
fn unset_immutable(path: &Path) -> Result<()> {
    let file = open(path, OFlags::RDWR, Mode::empty()).map_err(|e| {
        SboltError::EfiVar(format!(
            "failed to open efivarfs file: {}",
            std::io::Error::from(e)
        ))
    })?;

    // SAFETY: The opcode and payload type match Linux `FS_IOC_GETFLAGS`.
    let getter = unsafe { Getter::<FS_IOC_GETFLAGS, c_long>::new() };
    // SAFETY: The descriptor is open and the getter describes the expected ioctl ABI.
    let flags = unsafe { ioctl(&file, getter) }.map_err(|e| {
        SboltError::EfiVar(format!(
            "ioctl GETFLAGS failed: {}",
            std::io::Error::from(e)
        ))
    })?;

    if flags & FS_IMMUTABLE_FL == 0 {
        return Ok(());
    }

    // SAFETY: The opcode and payload type match Linux `FS_IOC_SETFLAGS`.
    let setter = unsafe { Setter::<FS_IOC_SETFLAGS, c_long>::new(flags & !FS_IMMUTABLE_FL) };
    // SAFETY: The descriptor is open and the setter describes the expected ioctl ABI.
    unsafe { ioctl(&file, setter) }.map_err(|e| {
        SboltError::EfiVar(format!(
            "ioctl SETFLAGS failed: {}",
            std::io::Error::from(e)
        ))
    })?;

    Ok(())
}

/// Create the Linux `efivarfs` mount point directory.
fn create_efivarfs_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)
        .map_err(|e| SboltError::EfiVar(format!("failed to create efivarfs mount point: {e}")))
}

/// Return the Linux `efivarfs` filename for a firmware variable.
fn variable_filename(id: &Id) -> String {
    format!("{}-{}", id.name(), id.vendor_guid())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::efi::guid::EFI_GLOBAL_VARIABLE;
    use crate::efi::variables::SECURE_BOOT;

    /// Test paths for the Linux backend.
    struct LinuxBackendTestContext {
        root: tempfile::TempDir,
        backend: Efivarfs,
    }

    impl LinuxBackendTestContext {
        /// Create a backend test context.
        fn new(name: &str, efi_boot: bool) -> Result<Self> {
            let root = tempfile::Builder::new()
                .prefix(&format!("sbolt-linux-{name}-"))
                .tempdir()?;
            let efi_firmware_path = root.path().join("efi");
            let efivarfs_path = efi_firmware_path.join("efivars");

            efi_boot
                .then(|| fs::create_dir_all(&efi_firmware_path))
                .transpose()?;

            let backend = Efivarfs {
                efi_firmware_path,
                efivarfs_path,
            };

            Ok(Self { root, backend })
        }

        /// Return the path for a variable in the test backend.
        fn variable_path(&self, id: &Id) -> PathBuf {
            self.backend.variable_path(id)
        }
    }

    /// Write an `efivarfs` test variable payload with a fake attributes header.
    fn write_var(root: &Path, id: &Id, payload: &[u8]) -> Result<()> {
        let path = root.join(variable_filename(id));
        let capacity = EFIVARFS_ATTRIBUTES_SIZE
            .checked_add(payload.len())
            .expect("test variable size overflow");
        let mut data = Vec::with_capacity(capacity);
        data.extend_from_slice(&[7_u8, 0, 0, 0]);
        data.extend_from_slice(payload);
        fs::write(path, data)?;

        Ok(())
    }

    #[test]
    fn mount_efivarfs_returns_false_when_not_efi_boot() {
        // ARRANGE
        let context = LinuxBackendTestContext::new("mount-not-efi", false).expect("test context");

        // ACT
        let mounted = context.backend.ensure_ready().expect("ensure ready");

        // ASSERT
        assert!(!mounted);
    }

    #[test]
    fn default_backend_uses_system_efi_paths() {
        // ARRANGE
        let backend = Efivarfs::new();
        let default_backend = Efivarfs::default();

        // ACT & ASSERT
        assert_eq!(backend.efi_firmware_path, PathBuf::from(EFI_FIRMWARE_PATH));
        assert_eq!(backend.efivarfs_path, PathBuf::from(EFIVARFS_PATH));
        assert_eq!(default_backend.efi_firmware_path, backend.efi_firmware_path);
        assert_eq!(default_backend.efivarfs_path, backend.efivarfs_path);
    }

    #[test]
    fn availability_and_variable_existence_follow_test_paths() {
        // ARRANGE
        let context = LinuxBackendTestContext::new("exists", true).expect("test context");

        // ACT
        let unavailable = context.backend.is_available();
        fs::create_dir_all(&context.backend.efivarfs_path).expect("create efivarfs dir");
        let available = context.backend.is_available();
        let missing_variable = context.backend.variable_exists(&SECURE_BOOT);
        write_var(&context.backend.efivarfs_path, &SECURE_BOOT, &[1]).expect("write variable");
        let existing_variable = context.backend.variable_exists(&SECURE_BOOT);

        // ASSERT
        assert!(!unavailable);
        assert!(available);
        assert!(!missing_variable);
        assert!(existing_variable);
    }

    #[test]
    fn read_variable_returns_none_when_file_is_missing() {
        // ARRANGE
        let context = LinuxBackendTestContext::new("missing-read", true).expect("test context");
        fs::create_dir_all(&context.backend.efivarfs_path).expect("create efivarfs dir");

        // ACT
        let payload = context
            .backend
            .read_variable(&SECURE_BOOT)
            .expect("read missing variable");

        // ASSERT
        assert!(payload.is_none());
    }

    #[test]
    fn variable_filename_formats_name_and_vendor_guid() {
        // ARRANGE
        let id = Id::new("PK", EFI_GLOBAL_VARIABLE);

        // ACT
        let filename = variable_filename(&id);

        // ASSERT
        assert_eq!(filename, format!("PK-{EFI_GLOBAL_VARIABLE}"));
    }

    #[test]
    fn mount_efivarfs_returns_true_when_already_available() {
        // ARRANGE
        let context = LinuxBackendTestContext::new("mount-existing", true).expect("test context");
        fs::create_dir_all(&context.backend.efivarfs_path).expect("create efivarfs dir");
        fs::write(
            context.backend.efivarfs_path.join(SECURE_BOOT_MARKER),
            [0_u8; EFIVARFS_ATTRIBUTES_SIZE + 1],
        )
        .expect("write marker");

        // ACT
        let mounted = context.backend.ensure_ready().expect("ensure ready");

        // ASSERT
        assert!(mounted);
    }

    #[test]
    fn mount_efivarfs_creates_directory_before_mount_attempt() {
        // ARRANGE
        let context = LinuxBackendTestContext::new("mount-create", true).expect("test context");

        // ACT
        let result = create_efivarfs_dir(&context.backend.efivarfs_path);

        // ASSERT
        assert!(context.backend.efivarfs_path.exists());
        result.expect("create efivarfs dir should succeed");
    }

    #[test]
    fn mount_efivarfs_surfaces_directory_creation_error() {
        // ARRANGE
        let context =
            LinuxBackendTestContext::new("mount-create-error", true).expect("test context");
        let blocker = context.root.path().join("blocker");
        fs::write(&blocker, b"not-a-directory").expect("write blocker");
        let backend = Efivarfs {
            efi_firmware_path: context.backend.efi_firmware_path.clone(),
            efivarfs_path: blocker.join("efivars"),
        };

        // ACT
        let result = backend.ensure_ready();

        // ASSERT
        result.expect_err("directory creation should fail");
    }

    #[test]
    fn mount_efivarfs_surfaces_mount_error() {
        // ARRANGE
        let context = LinuxBackendTestContext::new("mount-error", true).expect("test context");
        fs::create_dir_all(&context.backend.efivarfs_path).expect("create efivarfs dir");

        // ACT
        let result = context.backend.ensure_ready();

        // ASSERT
        let error = result.expect_err("mount should fail");
        assert!(format!("{error}").contains("failed to mount"));
    }

    #[test]
    fn read_variable_strips_efivarfs_attributes_header() {
        // ARRANGE
        let context = LinuxBackendTestContext::new("read", true).expect("test context");
        fs::create_dir_all(&context.backend.efivarfs_path).expect("create efivarfs dir");
        write_var(&context.backend.efivarfs_path, &SECURE_BOOT, &[1]).expect("write variable");

        // ACT
        let payload = context
            .backend
            .read_variable(&SECURE_BOOT)
            .expect("read variable")
            .expect("missing SecureBoot test variable");

        // ASSERT
        assert_eq!(payload, vec![1]);
    }

    #[test]
    fn read_variable_returns_none_for_short_payload() {
        // ARRANGE
        let context = LinuxBackendTestContext::new("short", true).expect("test context");
        fs::create_dir_all(&context.backend.efivarfs_path).expect("create efivarfs dir");
        fs::write(context.variable_path(&SECURE_BOOT), [1_u8, 2, 3]).expect("write variable");

        // ACT
        let payload = context
            .backend
            .read_variable(&SECURE_BOOT)
            .expect("read short variable");

        // ASSERT
        assert!(payload.is_none());
    }

    #[test]
    fn write_variable_writes_payload_to_expected_path() {
        // ARRANGE
        let context = LinuxBackendTestContext::new("write", true).expect("test context");
        fs::create_dir_all(&context.backend.efivarfs_path).expect("create efivarfs dir");
        let payload = b"signed-payload";

        // ACT
        context
            .backend
            .write_variable(Update::new(Id::new("PK", EFI_GLOBAL_VARIABLE), payload))
            .expect("write variable");
        let stored = fs::read(context.variable_path(&Id::new("PK", EFI_GLOBAL_VARIABLE)))
            .expect("read variable");

        // ASSERT
        assert_eq!(stored, payload);
    }

    #[test]
    fn write_variable_attempts_to_clear_existing_variable() {
        // ARRANGE
        let context = LinuxBackendTestContext::new("write-existing", true).expect("test context");
        fs::create_dir_all(&context.backend.efivarfs_path).expect("create efivarfs dir");
        let id = Id::new("PK", EFI_GLOBAL_VARIABLE);
        fs::write(context.variable_path(&id), b"existing").expect("write existing variable");

        // ACT
        let result = context
            .backend
            .write_variable(Update::new(id, b"replacement"));

        // ASSERT
        if let Err(error) = result {
            assert!(format!("{error}").contains("ioctl GETFLAGS failed"));
        }
    }

    #[test]
    fn unset_immutable_surfaces_missing_file_error() {
        // ARRANGE
        let context =
            LinuxBackendTestContext::new("immutable-missing", true).expect("test context");
        let missing_path = context.root.path().join("missing");

        // ACT
        let result = unset_immutable(&missing_path);

        // ASSERT
        result.expect_err("missing file should fail");
    }

    #[test]
    fn unset_immutable_attempts_regular_file_ioctl() {
        // ARRANGE
        let context = LinuxBackendTestContext::new("immutable-file", true).expect("test context");
        let path = context.root.path().join("variable");
        fs::write(&path, b"payload").expect("write variable file");

        // ACT
        let result = unset_immutable(&path);

        // ASSERT
        if let Err(error) = result {
            assert!(format!("{error}").contains("ioctl GETFLAGS failed"));
        }
    }
}
