//! Miso - Packages a Unified Kernel Image into a bootable image.

mod error;
mod fat;
mod img;
mod iso;

pub use error::MisoError;
pub use iso::SECTOR_SIZE;

/// Target architecture determining the EFI boot filename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    Aarch64,
}

impl Arch {
    /// Returns the UEFI fallback boot filename for this architecture.
    pub fn boot_filename(self) -> &'static str {
        match self {
            Arch::X86_64 => "BOOTX64.EFI",
            Arch::Aarch64 => "BOOTAA64.EFI",
        }
    }
}

/// A single file entry placed into a boot filesystem at the given path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    pub data: Vec<u8>,
}

/// Describes the boot filesystem layout for ISO and IMG generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootFsSpec {
    pub files: Vec<FileEntry>,
}

impl BootFsSpec {
    /// Constructs a [`BootFsSpec`] from a UKI blob and architecture.
    pub fn with_uki(arch: Arch, uki: Vec<u8>, extra_files: Vec<FileEntry>) -> Self {
        let uki_path = format!("EFI/BOOT/{}", arch.boot_filename());
        let mut files = Vec::with_capacity(1 + extra_files.len());
        files.push(FileEntry {
            path: uki_path,
            data: uki,
        });
        files.extend(extra_files);
        Self { files }
    }
}

/// Builds a bootable ISO 9660 image from a `BootFsSpec` into any `Write + Seek` sink.
pub fn build_iso(
    spec: &BootFsSpec,
    out: &mut (impl std::io::Write + std::io::Seek),
) -> Result<(), MisoError> {
    let efi_image = fat::build_efi_image(spec)?;
    iso::write_iso(out, &efi_image)
}

/// Builds a raw GPT disk image from a `BootFsSpec` into any `Read + Write + Seek` sink.
pub fn build_img(
    spec: &BootFsSpec,
    out: &mut (impl std::io::Read + std::io::Write + std::io::Seek),
) -> Result<(), MisoError> {
    let efi_image = fat::build_efi_image(spec)?;
    img::write_img(out, &efi_image)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn build_iso_bytes(spec: &BootFsSpec) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        build_iso(spec, &mut out).expect("build_iso must succeed");
        out.into_inner()
    }

    fn build_img_bytes(spec: &BootFsSpec) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        build_img(spec, &mut out).expect("build_img must succeed");
        out.into_inner()
    }

    #[test]
    fn arch_x86_64_boot_filename() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(Arch::X86_64.boot_filename(), "BOOTX64.EFI");
    }

    #[test]
    fn arch_aarch64_boot_filename() {
        // ARRANGE / ACT / ASSERT
        assert_eq!(Arch::Aarch64.boot_filename(), "BOOTAA64.EFI");
    }

    #[test]
    fn with_uki_places_uki_first_at_efi_boot_path() {
        // ARRANGE
        let uki = vec![0xABu8; 64];

        // ACT
        let spec = BootFsSpec::with_uki(Arch::X86_64, uki.clone(), vec![]);

        // ASSERT
        assert_eq!(spec.files.len(), 1);
        assert_eq!(spec.files[0].path, "EFI/BOOT/BOOTX64.EFI");
        assert_eq!(spec.files[0].data, uki);
    }

    #[test]
    fn with_uki_appends_extra_files_after_uki() {
        // ARRANGE
        let uki = vec![0u8; 32];
        let extra = FileEntry {
            path: "config.txt".to_owned(),
            data: b"arm_64bit=1".to_vec(),
        };

        // ACT
        let spec = BootFsSpec::with_uki(Arch::Aarch64, uki, vec![extra.clone()]);

        // ASSERT
        assert_eq!(spec.files.len(), 2);
        assert_eq!(spec.files[0].path, "EFI/BOOT/BOOTAA64.EFI");
        assert_eq!(spec.files[1], extra);
    }

    #[test]
    fn build_iso_returns_nonempty_image() {
        // ARRANGE
        let spec = BootFsSpec::with_uki(Arch::X86_64, vec![0xABu8; 1024], vec![]);

        // ACT
        let iso = build_iso_bytes(&spec);

        // ASSERT
        assert!(!iso.is_empty());
    }

    #[test]
    fn build_iso_output_has_cd001_magic() {
        // ARRANGE
        let spec = BootFsSpec::with_uki(Arch::X86_64, vec![0u8; 512], vec![]);

        // ACT
        let iso = build_iso_bytes(&spec);

        // ASSERT
        let pvd_offset = SECTOR_SIZE * 16 + 1;
        assert_eq!(&iso[pvd_offset..pvd_offset + 5], b"CD001");
    }

    #[test]
    fn build_iso_aarch64_produces_valid_iso() {
        // ARRANGE
        let spec = BootFsSpec::with_uki(Arch::Aarch64, vec![0xCCu8; 512], vec![]);

        // ACT
        let iso = build_iso_bytes(&spec);

        // ASSERT
        let pvd_offset = SECTOR_SIZE * 16 + 1;
        assert_eq!(&iso[pvd_offset..pvd_offset + 5], b"CD001");
    }

    #[test]
    fn build_img_returns_nonempty_image() {
        // ARRANGE
        let spec = BootFsSpec::with_uki(Arch::Aarch64, vec![0xABu8; 1024], vec![]);

        // ACT
        let img = build_img_bytes(&spec);

        // ASSERT
        assert!(!img.is_empty());
    }

    #[test]
    fn build_img_with_extra_files_succeeds() {
        // ARRANGE
        let spec = BootFsSpec::with_uki(
            Arch::Aarch64,
            vec![0xCCu8; 512],
            vec![FileEntry {
                path: "config.txt".to_owned(),
                data: b"arm_64bit=1\n".to_vec(),
            }],
        );

        // ACT
        let img = build_img_bytes(&spec);

        // ASSERT
        assert!(!img.is_empty());
    }

    #[test]
    fn build_iso_with_recursive_files_produces_valid_image() {
        // ARRANGE
        let spec = BootFsSpec::with_uki(
            Arch::X86_64,
            vec![0u8; 512],
            vec![FileEntry {
                path: "overlays/rpi/config.txt".to_owned(),
                data: b"arm_64bit=1".to_vec(),
            }],
        );

        // ACT
        let iso = build_iso_bytes(&spec);

        // ASSERT
        let pvd_offset = SECTOR_SIZE * 16 + 1;
        assert_eq!(&iso[pvd_offset..pvd_offset + 5], b"CD001");
    }
}
