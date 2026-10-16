//! Linux `efivarfs` variable storage.

use core::ffi::c_long;
use std::fs::{self, OpenOptions};
use std::io::Error as IoError;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use rustix::fs::{Mode, OFlags, open};
use rustix::ioctl::opcode::{read, write};
use rustix::ioctl::{Getter, Setter, ioctl};
use rustix::mount::{MountFlags, mount};

use super::Linux;
use crate::error::{FirmwareError, Result};
use crate::nvram::{Id, Update, VariableStore};

/// Size of the Linux `efivarfs` attribute header prepended to every payload.
const ATTRIBUTES_SIZE: usize = 4;

/// Marker file indicating `efivarfs` is mounted and usable.
const SECURE_BOOT_MARKER: &str = "SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c";

/// Linux `efivarfs` filesystem type name.
const EFIVARFS_TYPE: &str = "efivarfs";

/// `FS_IOC_GETFLAGS` opcode for Linux inode flags.
const FS_IOC_GETFLAGS: u32 = read::<c_long>(b'f', 1);

/// `FS_IOC_SETFLAGS` opcode for Linux inode flags.
const FS_IOC_SETFLAGS: u32 = write::<c_long>(b'f', 2);

/// Linux immutable inode flag.
const FS_IMMUTABLE_FL: c_long = 0x0000_0010;

impl VariableStore for Linux {
    fn is_boot(&self) -> bool {
        self.efi_firmware_path.exists()
    }

    fn is_available(&self) -> bool {
        self.efivarfs_path.exists()
    }

    fn prepare(&self) -> Result<bool> {
        mount_efivarfs(self)
    }

    fn exists(&self, id: &Id) -> bool {
        variable_path(self, id).exists()
    }

    fn read(&self, id: &Id) -> Result<Option<Vec<u8>>> {
        let path = variable_path(self, id);

        if !path.exists() {
            return Ok(None);
        }

        let data = fs::read(path)?;

        if data.len() < ATTRIBUTES_SIZE {
            return Ok(None);
        }

        let payload = data.get(ATTRIBUTES_SIZE..).ok_or_else(|| {
            FirmwareError::Variable("efivar payload missing attributes header".to_owned())
        })?;

        Ok(Some(payload.to_vec()))
    }

    fn write(&self, update: Update<'_>) -> Result<()> {
        let path = variable_path(self, update.id());

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

fn variable_path(store: &Linux, id: &Id) -> PathBuf {
    store.efivarfs_path.join(variable_filename(id))
}

fn mount_efivarfs(store: &Linux) -> Result<bool> {
    if !store.efi_firmware_path.exists() {
        return Ok(false);
    }

    if store.efivarfs_path.join(SECURE_BOOT_MARKER).exists() {
        return Ok(true);
    }

    if !store.efivarfs_path.exists() {
        create_efivarfs_dir(&store.efivarfs_path)?;
    }

    mount(
        EFIVARFS_TYPE,
        &store.efivarfs_path,
        EFIVARFS_TYPE,
        MountFlags::NOSUID | MountFlags::NOEXEC | MountFlags::NODEV,
        None,
    )
    .map_err(|e| FirmwareError::Variable(format!("failed to mount efivarfs: {e}")))?;

    Ok(true)
}

fn unset_immutable(path: &Path) -> Result<()> {
    let file = open(path, OFlags::RDWR, Mode::empty()).map_err(|e| {
        FirmwareError::Variable(format!(
            "failed to open efivarfs file: {}",
            IoError::from(e)
        ))
    })?;

    // SAFETY: The opcode and payload type match Linux `FS_IOC_GETFLAGS`.
    let getter = unsafe { Getter::<FS_IOC_GETFLAGS, c_long>::new() };
    // SAFETY: The descriptor is open and the getter describes the expected ioctl ABI.
    let flags = unsafe { ioctl(&file, getter) }.map_err(|e| {
        FirmwareError::Variable(format!("ioctl GETFLAGS failed: {}", IoError::from(e)))
    })?;

    if flags & FS_IMMUTABLE_FL == 0 {
        return Ok(());
    }

    // SAFETY: The opcode and payload type match Linux `FS_IOC_SETFLAGS`.
    let setter = unsafe { Setter::<FS_IOC_SETFLAGS, c_long>::new(flags & !FS_IMMUTABLE_FL) };
    // SAFETY: The descriptor is open and the setter describes the expected ioctl ABI.
    unsafe { ioctl(&file, setter) }.map_err(|e| {
        FirmwareError::Variable(format!("ioctl SETFLAGS failed: {}", IoError::from(e)))
    })?;

    Ok(())
}

fn create_efivarfs_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)
        .map_err(|e| FirmwareError::Variable(format!("failed to create efivarfs mount point: {e}")))
}

fn variable_filename(id: &Id) -> String {
    format!("{}-{}", id.name(), id.vendor_guid())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nvram;
    use crate::nvram::EFI_GLOBAL_VARIABLE;

    struct Context {
        root: tempfile::TempDir,
        store: Linux,
    }

    impl Context {
        fn new(name: &str, efi_boot: bool) -> Result<Self> {
            let root = tempfile::Builder::new()
                .prefix(&format!("firmware-{name}-"))
                .tempdir()?;
            let store = Linux::rooted(root.path());

            efi_boot
                .then(|| fs::create_dir_all(&store.efi_firmware_path))
                .transpose()?;

            Ok(Self { root, store })
        }

        fn variable_path(&self, id: &Id) -> PathBuf {
            variable_path(&self.store, id)
        }
    }

    fn sample() -> Id {
        Id::new("PK", EFI_GLOBAL_VARIABLE)
    }

    fn write_var(efivarfs_path: &Path, id: &Id, payload: &[u8]) -> Result<()> {
        let path = efivarfs_path.join(variable_filename(id));
        let capacity = ATTRIBUTES_SIZE
            .checked_add(payload.len())
            .expect("test variable size overflow");
        let mut data = Vec::with_capacity(capacity);
        data.extend_from_slice(&[7_u8, 0, 0, 0]);
        data.extend_from_slice(payload);
        fs::write(path, data)?;

        Ok(())
    }

    #[test]
    fn prepare_returns_false_when_not_efi_boot() {
        // ARRANGE
        let context = Context::new("not-efi", false).expect("test context");

        // ACT
        let prepared = context.store.prepare().expect("prepare");

        // ASSERT
        assert!(!prepared, "non-EFI systems must not mount efivarfs");
    }

    #[test]
    fn availability_and_existence_follow_test_paths() {
        // ARRANGE
        let context = Context::new("exists", true).expect("test context");

        // ACT
        let unavailable = context.store.is_available();
        fs::create_dir_all(&context.store.efivarfs_path).expect("create efivarfs dir");
        let available = context.store.is_available();
        let missing = context.store.exists(&sample());
        write_var(&context.store.efivarfs_path, &sample(), &[1]).expect("write variable");
        let present = context.store.exists(&sample());

        // ASSERT
        assert!(!unavailable, "efivarfs must not exist before creation");
        assert!(available, "efivarfs must exist after creation");
        assert!(!missing, "variable must be absent before writing");
        assert!(present, "variable must be present after writing");
    }

    #[test]
    fn read_returns_none_when_file_is_missing() {
        // ARRANGE
        let context = Context::new("missing-read", true).expect("test context");
        fs::create_dir_all(&context.store.efivarfs_path).expect("create efivarfs dir");

        // ACT
        let payload = context.store.read(&sample()).expect("read");

        // ASSERT
        assert!(payload.is_none(), "missing variables must read as none");
    }

    #[test]
    fn filename_formats_name_and_vendor_guid() {
        // ARRANGE
        let id = sample();

        // ACT
        let filename = variable_filename(&id);

        // ASSERT
        assert_eq!(filename, format!("PK-{EFI_GLOBAL_VARIABLE}"));
    }

    #[test]
    fn prepare_returns_true_when_marker_exists() {
        // ARRANGE
        let context = Context::new("prepare-existing", true).expect("test context");
        fs::create_dir_all(&context.store.efivarfs_path).expect("create efivarfs dir");
        fs::write(
            context.store.efivarfs_path.join(SECURE_BOOT_MARKER),
            [0_u8; ATTRIBUTES_SIZE],
        )
        .expect("write marker");

        // ACT
        let prepared = context.store.prepare().expect("prepare");

        // ASSERT
        assert!(prepared, "an existing marker means efivarfs is ready");
    }

    #[test]
    fn create_dir_creates_the_mount_point() {
        // ARRANGE
        let context = Context::new("mount-create", true).expect("test context");

        // ACT
        let result = create_efivarfs_dir(&context.store.efivarfs_path);

        // ASSERT
        assert!(
            context.store.efivarfs_path.exists(),
            "mount point must exist"
        );
        result.expect("create efivarfs dir should succeed");
    }

    #[test]
    fn create_dir_surfaces_errors() {
        // ARRANGE
        let context = Context::new("mount-create-error", true).expect("test context");
        let blocker = context.root.path().join("blocker");
        fs::write(&blocker, b"not-a-directory").expect("write blocker");

        // ACT
        let result = create_efivarfs_dir(&blocker.join("efivars"));

        // ASSERT
        result.expect_err("directory creation should fail");
    }

    #[test]
    fn read_strips_attributes_header() {
        // ARRANGE
        let context = Context::new("read", true).expect("test context");
        fs::create_dir_all(&context.store.efivarfs_path).expect("create efivarfs dir");
        write_var(&context.store.efivarfs_path, &sample(), &[1]).expect("write variable");

        // ACT
        let payload = context
            .store
            .read(&sample())
            .expect("read")
            .expect("missing variable");

        // ASSERT
        assert_eq!(payload, vec![1]);
    }

    #[test]
    fn read_returns_none_for_short_payload() {
        // ARRANGE
        let context = Context::new("short", true).expect("test context");
        fs::create_dir_all(&context.store.efivarfs_path).expect("create efivarfs dir");
        fs::write(context.variable_path(&sample()), [1_u8, 2, 3]).expect("write variable");

        // ACT
        let payload = context.store.read(&sample()).expect("read");

        // ASSERT
        assert!(payload.is_none(), "short payloads must read as none");
    }

    #[test]
    fn write_stores_payload_at_expected_path() {
        // ARRANGE
        let context = Context::new("write", true).expect("test context");
        fs::create_dir_all(&context.store.efivarfs_path).expect("create efivarfs dir");
        let payload = b"signed-payload";

        // ACT
        context
            .store
            .write(Update::new(sample(), payload))
            .expect("write variable");
        let stored = fs::read(context.variable_path(&sample())).expect("read variable");

        // ASSERT
        assert_eq!(stored, payload);
    }

    #[test]
    fn read_string_reads_and_decodes() {
        // ARRANGE
        let context = Context::new("string", true).expect("test context");
        fs::create_dir_all(&context.store.efivarfs_path).expect("create efivarfs dir");
        let mut payload: Vec<u8> = "Muak".encode_utf16().flat_map(u16::to_le_bytes).collect();
        payload.extend_from_slice(&[0, 0]);
        write_var(&context.store.efivarfs_path, &sample(), &payload).expect("write variable");

        // ACT
        let decoded = nvram::read_string(&context.store, &sample())
            .expect("read string")
            .expect("missing variable");

        // ASSERT
        assert_eq!(decoded, "Muak");
    }
}
