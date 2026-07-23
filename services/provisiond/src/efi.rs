//! EFI partition deployment of the Unified Kernel Image and overlay assets.

use std::fs;
use std::io;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rustix::fs::sync;

use crate::disk;

/// Mount point for the EFI partition during deployment.
pub const MOUNT_POINT: &str = "/run/mnt/efi";

/// Mounts the EFI partition at [`MOUNT_POINT`].
pub fn mount(efi_device: &str) -> Result<()> {
    if !Path::new(efi_device).exists() {
        bail!("EFI device {} does not exist", efi_device);
    }

    disk::mount_efi_partition(efi_device, MOUNT_POINT)
}

/// Syncs filesystem buffers and unmounts the EFI partition.
pub fn unmount() {
    sync();
    disk::try_unmount(MOUNT_POINT);
}

/// Opens a new file at `relative_path` under `esp_root` for writing.
/// Parent directories are created as needed. The path is validated
/// against directory traversal.
pub fn create(esp_root: &Path, relative_path: &str) -> Result<fs::File> {
    esp::path::validate_relative(relative_path)?;
    let dest = resolve(esp_root, relative_path);

    fs::File::create(&dest).with_context(|| format!("Failed to create {}", dest.display()))
}

/// Copies `size` bytes from `reader` into a file at `relative_path`
/// under `esp_root`. The path is validated against directory traversal.
pub fn write_file(
    esp_root: &Path,
    relative_path: &str,
    size: u64,
    reader: &mut dyn Read,
) -> Result<()> {
    esp::path::validate_relative(relative_path)?;
    let dest = resolve(esp_root, relative_path);

    copy_reader_to_file(reader, size, relative_path, &dest)
}

/// Extracts a streaming tar archive onto the ESP, creating one file per
/// archive entry under `esp_root`. Entry paths are joined as-is.
pub fn extract_tar(esp_root: &Path, reader: &mut dyn Read) -> Result<()> {
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

/// Writes `data` to a file at `relative_path` under `esp_root`.
/// The path is validated against directory traversal.
pub fn write_bytes(esp_root: &Path, relative_path: &str, data: &[u8]) -> Result<()> {
    esp::path::validate_relative(relative_path)?;
    let dest = resolve(esp_root, relative_path);

    fs::write(&dest, data).with_context(|| format!("Failed to write {}", dest.display()))
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
    let _ = create_parents(&dest);

    dest
}

fn copy_reader_to_file(
    reader: &mut dyn Read,
    size: u64,
    file_path: &str,
    dest: &Path,
) -> Result<()> {
    let mut writer =
        fs::File::create(dest).with_context(|| format!("Failed to create {}", dest.display()))?;
    let mut buf = [0_u8; 8192];
    let mut remaining = size;
    while remaining > 0 {
        let n = remaining.min(buf.len() as u64);
        let n = n as usize;
        let read = reader
            .read(buf.get_mut(..n).unwrap_or(&mut []))
            .context("Failed to read file data")?;
        if read == 0 {
            bail!("reader returned EOF before declared size: {file_path}");
        }
        writer
            .write_all(buf.get(..read).unwrap_or(&[]))
            .context("Failed to write file to EFI partition")?;
        remaining = remaining.saturating_sub(read as u64);
    }
    writer.flush()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

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
        let result = write_file(dir.path(), "../escape", data.len() as u64, &mut reader);

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
        write_file(dir.path(), "test.bin", data.len() as u64, &mut reader)
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
