//! FAT32 EFI System Partition image builder.

use std::io::{Cursor, Write as _};
use std::path::Path;

use fatfs::{Dir, FatType, FileSystem, FormatVolumeOptions, FsOptions};

use crate::{BootFsSpec, MisoError};

/// Minimum FAT32 image size in bytes.
const FAT_MIN_IMAGE_BYTES: usize = 1024 * 1024;

/// Flat overhead added on top of content size to reserve space for FAT metadata.
const FAT_METADATA_OVERHEAD: usize = 512 * 1024;

/// Rounds `n` up to the nearest multiple of `align`.
fn align_up(n: usize, align: usize) -> usize {
    (n + align - 1) & !(align - 1)
}

/// Builds an in-memory FAT32 EFI System Partition image from a `BootFsSpec`.
pub fn build_efi_image(spec: &BootFsSpec) -> Result<Vec<u8>, MisoError> {
    let total_content: usize = spec.files.iter().map(|e| e.data.len()).sum();
    let size = align_up(
        (total_content + FAT_METADATA_OVERHEAD).max(FAT_MIN_IMAGE_BYTES),
        512,
    );
    try_build_efi_image(size, spec)
}

/// Attempts to format and populate a FAT32 image of exactly `image_size` bytes.
fn try_build_efi_image(image_size: usize, spec: &BootFsSpec) -> Result<Vec<u8>, MisoError> {
    let buf = vec![0u8; image_size];
    let mut cursor = Cursor::new(buf);

    fatfs::format_volume(
        &mut cursor,
        FormatVolumeOptions::new()
            .volume_label(*b"EFI        ")
            .fat_type(FatType::Fat32),
    )
    .map_err(|e| MisoError::Fat(e.to_string()))?;

    {
        let fs = FileSystem::new(&mut cursor, FsOptions::new())
            .map_err(|e| MisoError::Fat(e.to_string()))?;

        let root = fs.root_dir();
        for entry in &spec.files {
            write_file_at_path(&root, &entry.path, &entry.data)?;
        }
    }

    Ok(cursor.into_inner())
}

/// Creates all intermediate directories and writes `data` at the given `path`.
fn write_file_at_path<'a, IO>(root: &Dir<'a, IO>, path: &str, data: &[u8]) -> Result<(), MisoError>
where
    IO: fatfs::ReadWriteSeek,
{
    let p = Path::new(path);
    let parent = p.parent();
    let filename = p
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| MisoError::Fat(format!("invalid file path: {path}")))?;

    let dir = match parent {
        None => root.clone(),
        Some(p) if p == Path::new("") => root.clone(),
        Some(parent) => open_or_create_dir(root, parent)?,
    };

    let mut file = dir
        .create_file(filename)
        .map_err(|e| MisoError::Fat(e.to_string()))?;
    file.write_all(data).map_err(MisoError::Io)?;
    Ok(())
}

/// Recursively opens or creates all components of `dir_path` under `root`.
fn open_or_create_dir<'a, IO>(root: &Dir<'a, IO>, dir_path: &Path) -> Result<Dir<'a, IO>, MisoError>
where
    IO: fatfs::ReadWriteSeek,
{
    let mut current = root.clone();
    for component in dir_path.components() {
        let name = component
            .as_os_str()
            .to_str()
            .ok_or_else(|| MisoError::Fat(format!("non-UTF-8 path component: {dir_path:?}")))?;
        current = current
            .create_dir(name)
            .map_err(|e| MisoError::Fat(e.to_string()))?;
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use super::*;
    use crate::{Arch, BootFsSpec, FileEntry};

    #[test]
    fn build_efi_image_returns_fat32_volume() {
        // ARRANGE
        let uki = b"fake-uki-payload";
        let spec = BootFsSpec::with_uki(Arch::X86_64, uki.to_vec(), vec![]);

        // ACT
        let image = build_efi_image(&spec).expect("should build FAT32 image");

        // ASSERT
        assert!(!image.is_empty(), "image must not be empty");
        let mut cursor = Cursor::new(image);
        let fs = FileSystem::new(&mut cursor, FsOptions::new())
            .expect("image should be a valid FAT filesystem");
        let root = fs.root_dir();
        let efi = root.open_dir("EFI").expect("EFI directory must exist");
        let boot = efi.open_dir("BOOT").expect("BOOT directory must exist");
        let mut file = boot.open_file("BOOTX64.EFI").expect("UKI file must exist");
        let mut content = Vec::new();
        file.read_to_end(&mut content)
            .expect("should read UKI file");
        assert_eq!(content, uki);
    }

    #[test]
    fn build_efi_image_size_is_sector_aligned() {
        // ARRANGE
        let spec = BootFsSpec::with_uki(Arch::X86_64, vec![0xABu8; 512], vec![]);

        // ACT
        let image = build_efi_image(&spec).expect("should succeed");

        // ASSERT
        assert_eq!(image.len() % 512, 0, "image size must be a multiple of 512");
    }

    #[test]
    fn build_efi_image_aarch64_boot_filename() {
        // ARRANGE
        let uki = b"arm-uki";
        let spec = BootFsSpec::with_uki(Arch::Aarch64, uki.to_vec(), vec![]);

        // ACT
        let image = build_efi_image(&spec).expect("should build FAT32 image");

        // ASSERT
        let mut cursor = Cursor::new(image);
        let fs = FileSystem::new(&mut cursor, FsOptions::new()).expect("valid FAT");
        let mut file = fs
            .root_dir()
            .open_dir("EFI")
            .expect("EFI dir must exist")
            .open_dir("BOOT")
            .expect("BOOT dir must exist")
            .open_file("BOOTAA64.EFI")
            .expect("BOOTAA64.EFI must exist");
        let mut content = Vec::new();
        file.read_to_end(&mut content).expect("read");
        assert_eq!(content, uki);
    }

    #[test]
    fn build_efi_image_minimum_size_at_least_fat_min() {
        // ARRANGE
        let spec = BootFsSpec::with_uki(Arch::X86_64, vec![0u8; 100], vec![]);

        // ACT
        let image = build_efi_image(&spec).expect("should succeed");

        // ASSERT
        assert!(
            image.len() >= FAT_MIN_IMAGE_BYTES,
            "image must be at least the minimum FAT image size"
        );
    }

    #[test]
    fn build_efi_image_with_extra_files_places_them_correctly() {
        // ARRANGE
        let uki = b"uki-payload";
        let blob_a = b"firmware-blob-a";
        let blob_b = b"firmware-blob-b";
        let spec = BootFsSpec::with_uki(
            Arch::Aarch64,
            uki.to_vec(),
            vec![
                FileEntry {
                    path: "START4.ELF".to_owned(),
                    data: blob_a.to_vec(),
                },
                FileEntry {
                    path: "FIXUP4.DAT".to_owned(),
                    data: blob_b.to_vec(),
                },
            ],
        );

        // ACT
        let image = build_efi_image(&spec).expect("should succeed");

        // ASSERT
        let mut cursor = Cursor::new(image);
        let fs = FileSystem::new(&mut cursor, FsOptions::new()).expect("valid FAT");
        let root = fs.root_dir();

        let mut uki_file = root
            .open_dir("EFI")
            .expect("EFI")
            .open_dir("BOOT")
            .expect("BOOT")
            .open_file("BOOTAA64.EFI")
            .expect("UKI must exist");
        let mut uki_content = Vec::new();
        uki_file.read_to_end(&mut uki_content).expect("read uki");
        assert_eq!(uki_content, uki);

        for (name, expected) in [("START4.ELF", blob_a as &[u8]), ("FIXUP4.DAT", blob_b)] {
            let mut f = root
                .open_file(name)
                .unwrap_or_else(|_| panic!("{name} must exist"));
            let mut content = Vec::new();
            f.read_to_end(&mut content).expect("read blob");
            assert_eq!(content, expected, "{name} content mismatch");
        }
    }

    #[test]
    fn build_efi_image_recursive_directory_support() {
        // ARRANGE
        let spec = BootFsSpec::with_uki(
            Arch::X86_64,
            b"uki".to_vec(),
            vec![FileEntry {
                path: "overlays/rpi/config.txt".to_owned(),
                data: b"arm_64bit=1".to_vec(),
            }],
        );

        // ACT
        let image = build_efi_image(&spec).expect("should succeed");

        // ASSERT
        let mut cursor = Cursor::new(image);
        let fs = FileSystem::new(&mut cursor, FsOptions::new()).expect("valid FAT");
        let mut file = fs
            .root_dir()
            .open_dir("overlays")
            .expect("overlays dir must exist")
            .open_dir("rpi")
            .expect("rpi dir must exist")
            .open_file("config.txt")
            .expect("config.txt must exist");
        let mut content = Vec::new();
        file.read_to_end(&mut content).expect("read");
        assert_eq!(content, b"arm_64bit=1");
    }

    #[test]
    fn try_build_efi_image_zero_size_returns_error() {
        // ARRANGE
        let spec = BootFsSpec::with_uki(Arch::X86_64, b"uki".to_vec(), vec![]);

        // ACT
        let result = try_build_efi_image(0, &spec);

        // ASSERT
        assert!(result.is_err());
    }

    fn make_fs_root(size: usize) -> Cursor<Vec<u8>> {
        let buf = vec![0u8; size];
        let mut cursor = Cursor::new(buf);
        fatfs::format_volume(
            &mut cursor,
            FormatVolumeOptions::new()
                .volume_label(*b"EFI        ")
                .fat_type(FatType::Fat32),
        )
        .expect("format");
        cursor
    }

    fn with_root<F: FnOnce(fatfs::Dir<'_, &mut Cursor<Vec<u8>>>)>(size: usize, f: F) {
        let mut cursor = make_fs_root(size);
        let fs = FileSystem::new(&mut cursor, FsOptions::new()).expect("open fs");
        f(fs.root_dir());
    }

    #[test]
    fn write_file_at_path_root_level_file() {
        // ARRANGE
        with_root(1024 * 1024, |root| {
            // ACT
            let result = write_file_at_path(&root, "flat.txt", b"hello");

            // ASSERT
            assert!(result.is_ok());
            let mut f = root.open_file("flat.txt").expect("file must exist");
            let mut content = Vec::new();
            f.read_to_end(&mut content).expect("read");
            assert_eq!(content, b"hello");
        });
    }

    #[test]
    fn write_file_at_path_dotdot_returns_error() {
        // ARRANGE
        with_root(1024 * 1024, |root| {
            // ACT
            let result = write_file_at_path(&root, "..", b"data");

            // ASSERT
            assert!(result.is_err());
            assert!(matches!(result.unwrap_err(), MisoError::Fat(_)));
        });
    }

    #[test]
    fn write_file_at_path_name_collides_with_dir_returns_error() {
        // ARRANGE
        with_root(1024 * 1024, |root| {
            root.create_dir("EFI").expect("create EFI dir");

            // ACT
            let result = write_file_at_path(&root, "EFI", b"data");

            // ASSERT
            assert!(result.is_err());
        });
    }

    #[test]
    fn open_or_create_dir_name_collides_with_file_returns_error() {
        // ARRANGE
        with_root(1024 * 1024, |root| {
            write_file_at_path(&root, "blob", b"x").expect("write file");

            // ACT
            let result = write_file_at_path(&root, "blob/sub.txt", b"data");

            // ASSERT
            assert!(result.is_err());
        });
    }

    #[test]
    fn align_up_already_aligned() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(align_up(512, 512), 512);
        assert_eq!(align_up(1024, 512), 1024);
    }

    #[test]
    fn align_up_rounds_up() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(align_up(513, 512), 1024);
        assert_eq!(align_up(1, 512), 512);
    }
}
