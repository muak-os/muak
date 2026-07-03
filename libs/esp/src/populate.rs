//! Mounted-directory population helpers for ESP contents.

use std::io::Write as _;
use std::path::Path;

use fatfs::error::FatError;

use crate::error::{EspError, Result};
use crate::model::EspFile;
use crate::path;

/// Streams each `EspFile.reader` into `<root>/<path>`.
///
/// # Errors
///
/// Returns an error when the spec contains invalid paths or the destination cannot be created or written.
pub fn write(files: &mut [EspFile<'_>], root: &Path) -> Result<()> {
    path::validate_spec_paths(files.iter().map(|file| file.path.as_str()))?;
    for file in files.iter_mut() {
        let rel_path = path::validate_relative_path(&file.path)?;
        let dest = root.join(rel_path);
        let mut parent = dest.clone();
        parent.pop();
        std::fs::create_dir_all(&parent)?;
        copy_to_file(file, &dest)?;
    }

    Ok(())
}

fn copy_to_file(file: &mut EspFile<'_>, dest: &Path) -> Result<()> {
    let mut writer = std::fs::File::create(dest)?;
    let mut buf = [0_u8; 8192];
    let mut remaining = file.size;
    while remaining > 0 {
        let n = usize::try_from(remaining.min(u64::try_from(buf.len()).unwrap_or(u64::MAX)))
            .unwrap_or(buf.len());
        let read = file
            .reader
            .read(buf.get_mut(..n).unwrap_or(&mut []))
            .map_err(EspError::Io)?;
        if read == 0 {
            return Err(FatError::Fat(format!(
                "reader returned EOF before declared size: {}",
                file.path
            ))
            .into());
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

    use super::write;
    use crate::model::{Arch, EspFile, EspSpec};

    fn fat_boot(data: &mut Cursor<Vec<u8>>) -> EspFile<'_> {
        let size = u64::try_from(data.get_ref().len()).unwrap_or(u64::MAX);

        EspFile::boot(Arch::X86_64, data, size)
    }

    #[test]
    fn populate_streams_files_into_mounted_dir() {
        // ARRANGE
        let dir = tempfile::tempdir().expect("temp dir must be created");
        let mut boot_data = Cursor::new(b"uki-payload".to_vec());
        let config_data = b"arm_64bit=1".to_vec();
        let config_size = u64::try_from(config_data.len()).unwrap_or(0);
        let mut config_cursor = Cursor::new(config_data);
        let mut spec = EspSpec::builder()
            .add_file(fat_boot(&mut boot_data))
            .expect("file must be added")
            .add_file(EspFile {
                path: "overlays/rpi/config.txt".to_owned(),
                reader: &mut config_cursor,
                size: config_size,
            })
            .expect("file must be added")
            .build()
            .expect("spec must build");

        // ACT
        write(spec.files_mut(), dir.path()).expect("populate must succeed");

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
        let mut boot_data = Cursor::new(b"uki".to_vec());
        let mut spec = EspSpec::builder()
            .add_file(fat_boot(&mut boot_data))
            .expect("file must be added")
            .build()
            .expect("spec must build");

        // ACT
        let result = write(spec.files_mut(), &read_only);

        // ASSERT
        assert!(result.is_err());
    }
}
