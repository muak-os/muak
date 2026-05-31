//! FAT32 ESP image builder.

use std::io::{Cursor, Write as _};
use std::path::Path;

use fatfs::{Dir, FileSystem, FsOptions};

use crate::error::{EspError, Result};
use crate::{EspSpec, format, path};

/// The minimum FAT32 ESP image size in bytes.
const FAT_MIN_IMAGE_BYTES: usize = 1024 * 1024;

/// The minimum ESP image growth step in bytes.
const FAT_GROWTH_MIN_BYTES: usize = 128 * 1024;

/// The maximum number of ESP image growth attempts.
const FAT_GROWTH_ATTEMPTS: usize = 16;

/// Builds a FAT32 ESP image from an `EspSpec`.
///
/// # Errors
///
/// Returns an error when the spec contains invalid paths or the FAT image cannot be
/// created within the configured growth attempts.
pub fn build(spec: &EspSpec) -> Result<Vec<u8>> {
    path::validate_spec(spec)?;
    let mut size = spec
        .total_file_bytes()
        .max(FAT_MIN_IMAGE_BYTES)
        .next_multiple_of(512);
    let mut last_error = None;

    for _ in 0..FAT_GROWTH_ATTEMPTS {
        match try_build(size, spec) {
            Ok(image) => return Ok(image),
            Err(err) => {
                last_error = Some(err);
                size = size
                    .saturating_add((size >> 4).max(FAT_GROWTH_MIN_BYTES))
                    .next_multiple_of(512);
            }
        }
    }

    if let Some(error) = last_error {
        Err(error)
    } else {
        Err(EspError::Fat("failed to size FAT image".to_owned()))
    }
}

/// Attempts to build an ESP image of exactly `image_size` bytes.
fn try_build(image_size: usize, spec: &EspSpec) -> Result<Vec<u8>> {
    let mut cursor = Cursor::new(vec![0_u8; image_size]);

    format(&mut cursor)?;

    {
        let fs = match FileSystem::new(&mut cursor, FsOptions::new()) {
            Ok(fs) => fs,
            Err(err) => return Err(EspError::Fat(err.to_string())),
        };
        let root = fs.root_dir();
        for file in &spec.files {
            write_file_at_path(&root, &file.path, &file.data)?;
        }
    }

    Ok(cursor.into_inner())
}

/// Creates intermediate FAT directories and writes one file.
pub(crate) fn write_file_at_path<IO>(root: &Dir<'_, IO>, path: &str, data: &[u8]) -> Result<()>
where
    IO: fatfs::ReadWriteSeek,
{
    let rel_path = path::validate_relative_path(path)?;
    let parent = rel_path.parent().unwrap_or(Path::new(""));
    let Some(filename) = rel_path.file_name().and_then(|name| name.to_str()) else {
        return Err(EspError::InvalidPath(format!("invalid file path: {path}")));
    };

    let dir = if parent == Path::new("") {
        root.clone()
    } else {
        open_or_create_dir(root, parent)?
    };

    let mut file = match dir.create_file(filename) {
        Ok(file) => file,
        Err(err) => return Err(EspError::Fat(err.to_string())),
    };
    file.write_all(data)?;
    Ok(())
}

/// Opens or creates all FAT directories beneath `root`.
fn open_or_create_dir<'a, IO>(root: &Dir<'a, IO>, dir_path: &Path) -> Result<Dir<'a, IO>>
where
    IO: fatfs::ReadWriteSeek,
{
    let mut current = root.clone();
    for component in dir_path.components() {
        let name = component.as_os_str().to_string_lossy();
        current = match current.create_dir(name.as_ref()) {
            Ok(next) => next,
            Err(err) => return Err(EspError::Fat(err.to_string())),
        };
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::io::Read as _;

    use fatfs::{FileSystem, FsOptions};

    use super::{FAT_MIN_IMAGE_BYTES, build, write_file_at_path};
    use crate::{Arch, EspError, EspFile, EspSpec, format};

    /// Creates a simple ESP spec for tests.
    fn test_spec() -> EspSpec {
        EspSpec::with_uki(
            Arch::X86_64,
            b"uki".to_vec(),
            vec![EspFile {
                path: "overlays/rpi/config.txt".to_owned(),
                data: b"arm_64bit=1".to_vec(),
            }],
        )
    }

    /// Opens a FAT filesystem from raw image bytes for one test closure.
    fn with_fs<F: FnOnce(fatfs::Dir<'_, &mut Cursor<Vec<u8>>>)>(image: Vec<u8>, test: F) {
        let mut cursor = Cursor::new(image);
        let fs = FileSystem::new(&mut cursor, FsOptions::new()).expect("FAT filesystem must open");
        test(fs.root_dir());
    }

    /// Creates a formatted FAT root directory for helper tests.
    fn with_root<F: FnOnce(fatfs::Dir<'_, &mut Cursor<Vec<u8>>>)>(size: usize, test: F) {
        let mut cursor = Cursor::new(vec![0_u8; size]);
        format(&mut cursor).expect("FAT format must succeed");
        let fs = FileSystem::new(&mut cursor, FsOptions::new()).expect("FAT open must succeed");
        test(fs.root_dir());
    }

    #[test]
    fn build_returns_fat32_image_with_boot_file() {
        // ARRANGE
        let spec = test_spec();

        // ACT
        let image = build(&spec).expect("ESP build must succeed");

        // ASSERT
        with_fs(image, |root| {
            let mut boot = root
                .open_dir("EFI")
                .expect("EFI dir must exist")
                .open_dir("BOOT")
                .expect("BOOT dir must exist")
                .open_file("BOOTX64.EFI")
                .expect("boot file must exist");
            let mut content = Vec::new();
            boot.read_to_end(&mut content).expect("boot file must read");
            assert_eq!(content, b"uki");
        });
    }

    #[test]
    fn build_preserves_recursive_extra_files() {
        // ARRANGE
        let spec = test_spec();

        // ACT
        let image = build(&spec).expect("ESP build must succeed");

        // ASSERT
        with_fs(image, |root| {
            let mut extra = root
                .open_dir("overlays")
                .expect("overlays dir must exist")
                .open_dir("rpi")
                .expect("rpi dir must exist")
                .open_file("config.txt")
                .expect("config file must exist");
            let mut content = Vec::new();
            extra
                .read_to_end(&mut content)
                .expect("config file must read");
            assert_eq!(content, b"arm_64bit=1");
        });
    }

    #[test]
    fn build_returns_sector_aligned_image() {
        // ARRANGE
        let spec = EspSpec::with_uki(Arch::X86_64, vec![0xAB; 1024], vec![]);

        // ACT
        let image = build(&spec).expect("ESP build must succeed");

        // ASSERT
        assert!(image.len().is_multiple_of(512));
        assert!(image.len() >= FAT_MIN_IMAGE_BYTES);
    }

    #[test]
    fn build_grows_large_payload_beyond_minimum_size() {
        // ARRANGE
        let spec = EspSpec::with_uki(Arch::X86_64, vec![0xAB; 96 * 1024 * 1024], vec![]);

        // ACT
        let image = build(&spec).expect("ESP build must succeed");

        // ASSERT
        assert!(image.len() > FAT_MIN_IMAGE_BYTES);
    }

    #[test]
    fn build_rejects_absolute_paths() {
        // ARRANGE
        let spec = EspSpec {
            files: vec![EspFile {
                path: "/EFI/BOOT/BOOTX64.EFI".to_owned(),
                data: b"uki".to_vec(),
            }],
        };

        // ACT
        let result = build(&spec);

        // ASSERT
        assert!(matches!(result, Err(EspError::InvalidPath(_))));
    }

    #[test]
    fn build_rejects_parent_traversal() {
        // ARRANGE
        let spec = EspSpec {
            files: vec![EspFile {
                path: "../escape".to_owned(),
                data: b"x".to_vec(),
            }],
        };

        // ACT
        let result = build(&spec);

        // ASSERT
        assert!(matches!(result, Err(EspError::InvalidPath(_))));
    }

    #[test]
    fn build_rejects_directory_only_paths() {
        // ARRANGE
        let spec = EspSpec {
            files: vec![EspFile {
                path: ".".to_owned(),
                data: b"x".to_vec(),
            }],
        };

        // ACT
        let result = build(&spec);

        // ASSERT
        assert!(matches!(result, Err(EspError::InvalidPath(_))));
    }

    #[test]
    fn build_propagates_file_creation_collisions() {
        // ARRANGE
        let spec = EspSpec {
            files: vec![
                EspFile {
                    path: "EFI".to_owned(),
                    data: b"file".to_vec(),
                },
                EspFile {
                    path: "EFI/BOOT/BOOTX64.EFI".to_owned(),
                    data: b"uki".to_vec(),
                },
            ],
        };

        // ACT
        let result = build(&spec);

        // ASSERT
        assert!(matches!(result, Err(EspError::Fat(_))));
    }

    #[test]
    fn write_file_at_path_writes_root_level_file() {
        // ARRANGE
        with_root(FAT_MIN_IMAGE_BYTES, |root| {
            // ACT
            let result = write_file_at_path(&root, "flat.txt", b"hello");

            // ASSERT
            result.expect("file must be written");
            let mut file = root.open_file("flat.txt").expect("file must exist");
            let mut content = Vec::new();
            file.read_to_end(&mut content).expect("file must read");
            assert_eq!(content, b"hello");
        });
    }

    #[test]
    fn write_file_at_path_writes_nested_file() {
        // ARRANGE
        with_root(FAT_MIN_IMAGE_BYTES, |root| {
            // ACT
            let result = write_file_at_path(&root, "nested/tree/file.txt", b"hello");

            // ASSERT
            result.expect("file must be written");
            let mut file = root
                .open_dir("nested")
                .expect("nested dir must exist")
                .open_dir("tree")
                .expect("tree dir must exist")
                .open_file("file.txt")
                .expect("file must exist");
            let mut content = Vec::new();
            file.read_to_end(&mut content).expect("file must read");
            assert_eq!(content, b"hello");
        });
    }

    #[test]
    fn write_file_at_path_rejects_dotdot() {
        // ARRANGE
        with_root(FAT_MIN_IMAGE_BYTES, |root| {
            // ACT
            let result = write_file_at_path(&root, "../blob", b"data");

            // ASSERT
            assert!(matches!(result, Err(EspError::InvalidPath(_))));
        });
    }

    #[test]
    fn open_or_create_dir_errors_when_path_collides_with_file() {
        // ARRANGE
        with_root(FAT_MIN_IMAGE_BYTES, |root| {
            write_file_at_path(&root, "blob", b"x").expect("seed file must be created");

            // ACT
            let result = write_file_at_path(&root, "blob/sub.txt", b"data");

            // ASSERT
            assert!(result.is_err());
        });
    }
}
