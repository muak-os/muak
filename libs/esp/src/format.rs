//! FAT32 EFI volume formatting helpers.

use fatfs::{FatType, FormatVolumeOptions};

use crate::EspError;

/// The EFI FAT volume label padded to 11 bytes.
const FAT_VOLUME_LABEL: [u8; 11] = *b"EFI        ";

/// Formats any readable and writable target as an EFI FAT32 volume.
pub fn format<IO>(io: &mut IO) -> Result<(), EspError>
where
    IO: fatfs::ReadWriteSeek,
{
    fatfs::format_volume(
        io,
        FormatVolumeOptions::new()
            .volume_label(FAT_VOLUME_LABEL)
            .fat_type(FatType::Fat32),
    )
    .map_err(|err| EspError::Fat(err.to_string()))
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read, Seek as _, SeekFrom, Write};

    use fatfs::{FileSystem, FsOptions};

    use super::format;
    use crate::EspError;

    struct BrokenIo;

    impl Read for BrokenIo {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("read failed"))
        }
    }

    impl Write for BrokenIo {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("write failed"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl std::io::Seek for BrokenIo {
        fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
            Err(std::io::Error::other("seek failed"))
        }
    }

    #[test]
    fn format_creates_efi_fat_volume() {
        // ARRANGE
        let mut cursor = Cursor::new(vec![0u8; 1024 * 1024]);

        // ACT
        format(&mut cursor).expect("format must succeed");

        // ASSERT
        let fs = FileSystem::new(&mut cursor, FsOptions::new()).expect("FAT filesystem must open");
        assert_eq!(fs.volume_label_as_bytes(), b"EFI");
    }

    #[test]
    fn format_resets_existing_data_to_empty_root() {
        // ARRANGE
        let mut cursor = Cursor::new(vec![0u8; 1024 * 1024]);
        format(&mut cursor).expect("initial format must succeed");
        {
            let fs =
                FileSystem::new(&mut cursor, FsOptions::new()).expect("FAT filesystem must open");
            fs.root_dir()
                .create_file("stale.txt")
                .expect("stale file must be created");
        }

        // ACT
        cursor.seek(SeekFrom::Start(0)).expect("seek must succeed");
        format(&mut cursor).expect("reformat must succeed");

        // ASSERT
        let fs = FileSystem::new(&mut cursor, FsOptions::new()).expect("FAT filesystem must open");
        assert!(fs.root_dir().open_file("stale.txt").is_err());
    }

    #[test]
    fn format_wraps_fat_errors() {
        // ARRANGE
        let mut io = BrokenIo;

        // ACT
        let result = format(&mut io);

        // ASSERT
        assert!(matches!(result, Err(EspError::Fat(_))));
    }

    #[test]
    fn broken_io_methods_fail_consistently() {
        // ARRANGE
        let mut io = BrokenIo;
        let mut buffer = [0u8; 4];

        // ACT / ASSERT
        assert!(io.read(&mut buffer).is_err());
        assert!(io.write(b"data").is_err());
        assert!(io.flush().is_ok());
        assert!(io.seek(SeekFrom::Start(0)).is_err());
    }
}
