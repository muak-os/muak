//! Mounted-directory population helpers for ESP contents.

use std::io::{Read, Write as _};
use std::path::Path;

use fatfs::error::FatError;

use crate::FileMeta;
use crate::error::{EspError, Result};
use crate::path;

/// Streams file data into `<root>/<path>` for each file.
///
/// # Errors
///
/// Returns an error when the paths are invalid or the destination cannot be created or written.
pub fn write(files: &[FileMeta<'_>], readers: &mut [&mut dyn Read], root: &Path) -> Result<()> {
    if files.len() != readers.len() {
        return Err(EspError::InvalidOrder(format!(
            "files count ({}) doesn't match readers count ({})",
            files.len(),
            readers.len()
        )));
    }

    path::validate_spec_paths(files.iter().map(|file| file.path))?;

    for (file, reader) in files.iter().zip(readers.iter_mut()) {
        let rel_path = path::validate_relative_path(file.path)?;
        let dest = root.join(rel_path);
        let mut parent = dest.clone();
        parent.pop();
        std::fs::create_dir_all(&parent)?;
        copy_to_file(reader, file.size, file.path, &dest)?;
    }

    Ok(())
}

fn copy_to_file(reader: &mut dyn Read, size: u64, file_path: &str, dest: &Path) -> Result<()> {
    let mut writer = std::fs::File::create(dest)?;
    let mut buf = [0_u8; 8192];
    let mut remaining = size;
    while remaining > 0 {
        let n = usize::try_from(remaining.min(u64::try_from(buf.len()).unwrap_or(u64::MAX)))
            .unwrap_or(buf.len());
        let read = reader
            .read(buf.get_mut(..n).unwrap_or(&mut []))
            .map_err(EspError::Io)?;
        if read == 0 {
            return Err(EspError::Fat(FatError::Fat(format!(
                "reader returned EOF before declared size: {file_path}"
            ))));
        }
        writer
            .write_all(buf.get(..read).unwrap_or(&[]))
            .map_err(EspError::Io)?;
        let read_u64 = u64::try_from(read).unwrap_or(0);
        remaining = remaining.saturating_sub(read_u64);
    }
    writer.flush()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::os::unix::fs::PermissionsExt as _;

    use fatfs::types::FileMeta;

    use super::write;

    #[test]
    fn populate_streams_files_into_mounted_dir() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir must be created");
        let boot_data = b"uki-payload";
        let config_data = b"arm_64bit=1";
        let mut boot_reader = Cursor::new(boot_data.as_slice());
        let mut config_reader = Cursor::new(config_data.as_slice());
        let files = &[
            FileMeta::new(
                "EFI/BOOT/BOOTX64.EFI",
                u64::try_from(boot_data.len()).unwrap_or(0),
            ),
            FileMeta::new(
                "overlays/rpi/config.txt",
                u64::try_from(config_data.len()).unwrap_or(0),
            ),
        ];
        let mut readers: Vec<&mut dyn std::io::Read> = vec![&mut boot_reader, &mut config_reader];

        // ACT
        write(files, &mut readers, dir.path()).expect("populate must succeed");

        // ASSERT
        assert_eq!(
            std::fs::read(dir.path().join("EFI/BOOT/BOOTX64.EFI")).expect("boot file must exist"),
            b"uki-payload"
        );
        assert_eq!(
            std::fs::read(dir.path().join("overlays/rpi/config.txt"))
                .expect("config file must exist"),
            b"arm_64bit=1"
        );
    }

    #[test]
    fn populate_propagates_write_errors() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir must be created");
        let read_only = dir.path().join("readonly");
        std::fs::create_dir_all(&read_only).expect("dir must be created");
        std::fs::set_permissions(&read_only, std::fs::Permissions::from_mode(0o555))
            .expect("permissions must be set");
        let boot_data = b"uki";
        let mut boot_reader = Cursor::new(boot_data.as_slice());
        let files = &[FileMeta::new(
            "EFI/BOOT/BOOTX64.EFI",
            u64::try_from(boot_data.len()).unwrap_or(0),
        )];
        let mut readers: Vec<&mut dyn std::io::Read> = vec![&mut boot_reader];

        // ACT
        let result = write(files, &mut readers, &read_only);

        // ASSERT
        assert!(result.is_err());
    }
}
