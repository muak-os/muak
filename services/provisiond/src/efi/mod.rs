//! EFI partition deployment of the Unified Kernel Image and overlay assets.

mod atomic;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::{fs, io};

use anyhow::{Context as _, Result, bail};
use esp::path::validate_relative;
use rustix::fs::sync;

use crate::disk;

/// Mount point for the EFI partition during deployment.
pub(crate) const MOUNT_POINT: &str = "/run/mnt/efi";

/// Mounts the EFI partition at [`MOUNT_POINT`].
pub(crate) fn mount(efi_device: &str) -> Result<()> {
    if !Path::new(efi_device).exists() {
        bail!("EFI device {efi_device} does not exist");
    }

    disk::mount::efi(efi_device, MOUNT_POINT)
}

/// Syncs filesystem buffers and unmounts the EFI partition.
pub(crate) fn unmount() {
    sync();
    disk::mount::try_unmount(MOUNT_POINT);
}

/// Opens a new file at `relative_path` under `esp_root` for writing.
pub(crate) fn create(esp_root: &Path, relative_path: &str) -> Result<fs::File> {
    validate_relative(relative_path)?;
    let dest = resolve(esp_root, relative_path);

    fs::File::create(&dest).with_context(|| format!("Failed to create {}", dest.display()))
}

/// Atomically copies bytes from `reader` into a file at `relative_path` under `esp_root`.
pub(crate) fn write_file(
    esp_root: &Path,
    relative_path: &str,
    size: u64,
    reader: &mut dyn Read,
) -> Result<()> {
    atomic::write_file(esp_root, relative_path, size, reader)
}

/// Extracts a streaming tar archive onto the ESP.
pub(crate) fn extract_tar(esp_root: &Path, reader: &mut dyn Read) -> Result<()> {
    let mut archive = tar::Archive::new(reader);
    for entry in archive.entries().context("iterate tar entries")? {
        let mut entry = entry.context("read tar entry")?;
        let path = entry.path().context("read tar entry path")?;
        let dest = esp_root.join(path.as_ref());
        create_parents(&dest)?;
        let mut file = fs::File::create(&dest)
            .with_context(|| format!("Failed to create {}", dest.display()))?;
        io::copy(&mut entry, &mut file)
            .with_context(|| format!("Failed to write {}", dest.display()))?;
    }

    Ok(())
}

/// Atomically writes `data` to a file at `relative_path` under `esp_root`.
pub(crate) fn write_bytes(esp_root: &Path, relative_path: &str, data: &[u8]) -> Result<()> {
    atomic::write_bytes(esp_root, relative_path, data)
}

fn create_parents(dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("Failed to create parent directories for {}", dest.display())
        })?;
    }

    Ok(())
}

fn resolve(esp_root: &Path, relative_path: &str) -> PathBuf {
    let dest = esp_root.join(relative_path);
    let _created = create_parents(&dest);

    dest
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write as _};

    use super::*;

    #[test]
    fn mount_rejects_nonexistent_device() {
        // ACT
        let result = mount("/nonexistent/efi");

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn write_file_rejects_parent_traversal() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir must be created");
        let data = b"uki";
        let mut reader = Cursor::new(data.as_slice());

        // ACT
        let result = write_file(
            dir.path(),
            "../escape",
            u64::try_from(data.len()).unwrap_or(0),
            &mut reader,
        );

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn write_file_copies_all_bytes() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir must be created");
        let data = b"hello from efi write";
        let mut reader = Cursor::new(data.as_slice());

        // ACT
        write_file(
            dir.path(),
            "test.bin",
            u64::try_from(data.len()).unwrap_or(0),
            &mut reader,
        )
        .expect("write_file must succeed");

        // ASSERT
        assert_eq!(
            std::fs::read(dir.path().join("test.bin")).expect("file must exist"),
            data
        );
    }

    #[test]
    fn write_file_rejects_early_eof() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir must be created");
        let data = b"short";
        let mut reader = Cursor::new(data.as_slice());

        // ACT
        let result = write_file(dir.path(), "partial.bin", 100, &mut reader);

        // ASSERT
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("EOF"));
    }

    #[test]
    fn extract_tar_writes_files() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir must be created");
        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            let mut header = tar::Header::new_gnu();
            header.set_path("EFI/BOOT/BOOTX64.EFI").unwrap();
            header.set_size(3);
            header.set_mode(0o644);
            builder
                .append_data(&mut header, "EFI/BOOT/BOOTX64.EFI", Cursor::new(b"uki"))
                .unwrap();
            let mut header = tar::Header::new_gnu();
            header.set_path("config.txt").unwrap();
            header.set_size(6);
            header.set_mode(0o644);
            builder
                .append_data(&mut header, "config.txt", Cursor::new(b"config"))
                .unwrap();
            builder.finish().unwrap();
        }

        // ACT
        let mut reader = Cursor::new(&tar_bytes);
        extract_tar(dir.path(), &mut reader).expect("extract_tar must succeed");

        // ASSERT
        assert_eq!(
            std::fs::read(dir.path().join("EFI/BOOT/BOOTX64.EFI")).expect("uki must exist"),
            b"uki"
        );
        assert_eq!(
            std::fs::read(dir.path().join("config.txt")).expect("config must exist"),
            b"config"
        );
    }

    #[test]
    fn write_bytes_writes_file() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir must be created");

        // ACT
        write_bytes(dir.path(), "luks", b"secret-key").expect("write_bytes must succeed");

        // ASSERT
        assert_eq!(
            std::fs::read(dir.path().join("luks")).expect("luks file must exist"),
            b"secret-key"
        );
    }

    #[test]
    fn create_opens_writable_file() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir must be created");

        // ACT
        let mut file = create(dir.path(), "EFI/BOOT/BOOTX64.EFI").expect("create must succeed");
        file.write_all(b"payload").unwrap();
        drop(file);

        // ASSERT
        assert_eq!(
            std::fs::read(dir.path().join("EFI/BOOT/BOOTX64.EFI")).expect("file must exist"),
            b"payload"
        );
    }
}
