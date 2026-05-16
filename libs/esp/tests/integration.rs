use std::io::Cursor;

use esp::{Arch, EspFile, EspSpec};
use fatfs::{FileSystem, FsOptions};

#[test]
fn public_api_build_and_populate_match() {
    // ARRANGE
    let spec = EspSpec::with_uki(
        Arch::X86_64,
        b"uki-payload".to_vec(),
        vec![EspFile {
            path: "overlays/rpi/config.txt".to_owned(),
            data: b"arm_64bit=1".to_vec(),
        }],
    );
    let dest = tempfile::tempdir().expect("temp dir must be created");

    // ACT
    let image = esp::build(&spec).expect("build must succeed");
    esp::populate(&spec, dest.path()).expect("populate must succeed");

    // ASSERT
    let mut cursor = Cursor::new(image);
    let fs = FileSystem::new(&mut cursor, FsOptions::new()).expect("FAT filesystem must open");
    let mut boot = fs
        .root_dir()
        .open_dir("EFI")
        .expect("EFI dir must exist")
        .open_dir("BOOT")
        .expect("BOOT dir must exist")
        .open_file("BOOTX64.EFI")
        .expect("boot file must exist");
    let mut boot_bytes = Vec::new();
    std::io::Read::read_to_end(&mut boot, &mut boot_bytes).expect("boot file must read");
    assert_eq!(boot_bytes, b"uki-payload");
    assert_eq!(
        std::fs::read(dest.path().join("EFI/BOOT/BOOTX64.EFI")).expect("boot file must exist"),
        boot_bytes
    );
    assert_eq!(
        std::fs::read(dest.path().join("overlays/rpi/config.txt")).expect("config file must exist"),
        b"arm_64bit=1"
    );
}

#[test]
fn public_api_format_then_build_outputs_readable_fat_images() {
    // ARRANGE
    let spec = EspSpec::with_uki(Arch::Aarch64, b"uki".to_vec(), vec![]);
    let mut device = Cursor::new(vec![0u8; 1024 * 1024]);

    // ACT
    esp::format(&mut device).expect("format must succeed");
    let image = esp::build(&spec).expect("build must succeed");

    // ASSERT
    let fs = FileSystem::new(&mut device, FsOptions::new()).expect("formatted device must open");
    assert_eq!(fs.volume_label_as_bytes(), b"EFI");

    let mut image_cursor = Cursor::new(image);
    let image_fs =
        FileSystem::new(&mut image_cursor, FsOptions::new()).expect("image must open as FAT");
    let mut boot = image_fs
        .root_dir()
        .open_dir("EFI")
        .expect("EFI dir must exist")
        .open_dir("BOOT")
        .expect("BOOT dir must exist")
        .open_file("BOOTAA64.EFI")
        .expect("boot file must exist");
    let mut boot_bytes = Vec::new();
    std::io::Read::read_to_end(&mut boot, &mut boot_bytes).expect("boot file must read");
    assert_eq!(boot_bytes, b"uki");
}
