//! Linux `efivarfs` firmware variable backend.

use core::ffi::c_long;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use rustix::fs::{Mode, OFlags, open};
use rustix::ioctl::opcode::{read, write};
use rustix::ioctl::{Getter, Setter, ioctl};
use rustix::mount::{MountFlags, mount};

use super::{FirmwareVariableBackend, FirmwareVariableId, FirmwareVariableUpdate};
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
pub(crate) struct LinuxFirmwareVariables {
    efi_firmware_path: PathBuf,
    efivarfs_path: PathBuf,
}

impl LinuxFirmwareVariables {
    /// Create the default Linux firmware variable backend.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            efi_firmware_path: PathBuf::from(EFI_FIRMWARE_PATH),
            efivarfs_path: PathBuf::from(EFIVARFS_PATH),
        }
    }

    /// Return the filesystem path for a firmware variable.
    fn variable_path(&self, id: &FirmwareVariableId) -> PathBuf {
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

impl Default for LinuxFirmwareVariables {
    /// Create the default Linux firmware variable backend.
    fn default() -> Self {
        Self::new()
    }
}

impl FirmwareVariableBackend for LinuxFirmwareVariables {
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
    fn variable_exists(&self, id: &FirmwareVariableId) -> bool {
        self.variable_path(id).exists()
    }

    /// Read an `efivarfs` payload without the Linux attributes header.
    fn read_variable(&self, id: &FirmwareVariableId) -> Result<Option<Vec<u8>>> {
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
    fn write_variable(&self, update: FirmwareVariableUpdate<'_>) -> Result<()> {
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
fn variable_filename(id: &FirmwareVariableId) -> String {
    format!("{}-{}", id.name(), id.vendor_guid())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::efi::guid::EFI_GLOBAL_VARIABLE;
    use crate::platform::SECURE_BOOT_VARIABLE;

    /// Test paths for the Linux backend.
    struct LinuxBackendTestContext {
        root: tempfile::TempDir,
        backend: LinuxFirmwareVariables,
    }

    impl LinuxBackendTestContext {
        /// Create a backend test context.
        fn new(name: &str, efi_boot: bool) -> Result<Self> {
            let root = tempfile::Builder::new()
                .prefix(&format!("sbolt-linux-{name}-"))
                .tempdir()?;
            let efi_firmware_path = root.path().join("efi");
            let efivarfs_path = efi_firmware_path.join("efivars");

            if efi_boot {
                fs::create_dir_all(&efi_firmware_path)?;
            }

            let backend = LinuxFirmwareVariables {
                efi_firmware_path,
                efivarfs_path,
            };

            Ok(Self { root, backend })
        }

        /// Return the path for a variable in the test backend.
        fn variable_path(&self, id: &FirmwareVariableId) -> PathBuf {
            self.backend.variable_path(id)
        }
    }

    /// Write an `efivarfs` test variable payload with a fake attributes header.
    fn write_var(root: &Path, id: &FirmwareVariableId, payload: &[u8]) -> Result<()> {
        let path = root.join(variable_filename(id));
        let mut data = Vec::with_capacity(EFIVARFS_ATTRIBUTES_SIZE + payload.len());
        data.extend_from_slice(&[7_u8, 0, 0, 0]);
        data.extend_from_slice(payload);
        fs::write(path, data)?;

        Ok(())
    }

    #[test]
    fn mount_efivarfs_returns_false_when_not_efi_boot() -> Result<()> {
        // ARRANGE
        let context = LinuxBackendTestContext::new("mount-not-efi", false)?;

        // ACT
        let mounted = context.backend.ensure_ready()?;

        // ASSERT
        assert!(!mounted);

        Ok(())
    }

    #[test]
    fn mount_efivarfs_returns_true_when_already_available() -> Result<()> {
        // ARRANGE
        let context = LinuxBackendTestContext::new("mount-existing", true)?;
        fs::create_dir_all(&context.backend.efivarfs_path)?;
        fs::write(
            context.backend.efivarfs_path.join(SECURE_BOOT_MARKER),
            [0_u8; EFIVARFS_ATTRIBUTES_SIZE + 1],
        )?;

        // ACT
        let mounted = context.backend.ensure_ready()?;

        // ASSERT
        assert!(mounted);

        Ok(())
    }

    #[test]
    fn mount_efivarfs_creates_directory_before_mount_attempt() -> Result<()> {
        // ARRANGE
        let context = LinuxBackendTestContext::new("mount-create", true)?;

        // ACT
        let result = create_efivarfs_dir(&context.backend.efivarfs_path);

        // ASSERT
        assert!(context.backend.efivarfs_path.exists());
        assert!(result.is_ok());

        Ok(())
    }

    #[test]
    fn mount_efivarfs_surfaces_directory_creation_error() -> Result<()> {
        // ARRANGE
        let context = LinuxBackendTestContext::new("mount-create-error", true)?;
        let blocker = context.root.path().join("blocker");
        fs::write(&blocker, b"not-a-directory")?;
        let backend = LinuxFirmwareVariables {
            efi_firmware_path: context.backend.efi_firmware_path.clone(),
            efivarfs_path: blocker.join("efivars"),
        };

        // ACT
        let result = backend.ensure_ready();

        // ASSERT
        assert!(result.is_err());

        Ok(())
    }

    #[test]
    fn read_variable_strips_efivarfs_attributes_header() -> Result<()> {
        // ARRANGE
        let context = LinuxBackendTestContext::new("read", true)?;
        fs::create_dir_all(&context.backend.efivarfs_path)?;
        write_var(&context.backend.efivarfs_path, &SECURE_BOOT_VARIABLE, &[1])?;

        // ACT
        let payload = context
            .backend
            .read_variable(&SECURE_BOOT_VARIABLE)?
            .ok_or_else(|| SboltError::EfiVar("missing SecureBoot test variable".into()))?;

        // ASSERT
        assert_eq!(payload, vec![1]);

        Ok(())
    }

    #[test]
    fn read_variable_returns_none_for_short_payload() -> Result<()> {
        // ARRANGE
        let context = LinuxBackendTestContext::new("short", true)?;
        fs::create_dir_all(&context.backend.efivarfs_path)?;
        fs::write(context.variable_path(&SECURE_BOOT_VARIABLE), [1_u8, 2, 3])?;

        // ACT
        let payload = context.backend.read_variable(&SECURE_BOOT_VARIABLE)?;

        // ASSERT
        assert!(payload.is_none());

        Ok(())
    }

    #[test]
    fn write_variable_writes_payload_to_expected_path() -> Result<()> {
        // ARRANGE
        let context = LinuxBackendTestContext::new("write", true)?;
        fs::create_dir_all(&context.backend.efivarfs_path)?;
        let payload = b"signed-payload";

        // ACT
        context.backend.write_variable(FirmwareVariableUpdate::new(
            FirmwareVariableId::new("PK", EFI_GLOBAL_VARIABLE),
            payload,
        ))?;
        let stored =
            fs::read(context.variable_path(&FirmwareVariableId::new("PK", EFI_GLOBAL_VARIABLE)))?;

        // ASSERT
        assert_eq!(stored, payload);

        Ok(())
    }
}
