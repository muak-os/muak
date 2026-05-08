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
    pub boot_filename: String,
    pub uki: Vec<u8>,
    pub files: Vec<FileEntry>,
}

/// Builds a bootable ISO 9660 image in memory from a `BootFsSpec`.
pub fn build_iso(spec: &BootFsSpec) -> Result<Vec<u8>, MisoError> {
    let efi_image = fat::build_efi_image(spec)?;
    let mut out = std::io::Cursor::new(Vec::new());
    iso::write_iso(&mut out, &efi_image)?;
    Ok(out.into_inner())
}

/// Builds a raw GPT disk image in memory from a `BootFsSpec`.
pub fn build_img(spec: &BootFsSpec) -> Result<Vec<u8>, MisoError> {
    let efi_image = fat::build_efi_image(spec)?;
    let mut out = std::io::Cursor::new(Vec::new());
    img::write_img(&mut out, &efi_image)?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn build_iso_returns_nonempty_image() {
        // ARRANGE
        let spec = BootFsSpec {
            boot_filename: Arch::X86_64.boot_filename().to_owned(),
            uki: vec![0xABu8; 1024],
            files: vec![],
        };

        // ACT
        let iso = build_iso(&spec).expect("build_iso must succeed");

        // ASSERT
        assert!(!iso.is_empty());
    }

    #[test]
    fn build_iso_output_has_cd001_magic() {
        // ARRANGE
        let spec = BootFsSpec {
            boot_filename: Arch::X86_64.boot_filename().to_owned(),
            uki: vec![0u8; 512],
            files: vec![],
        };

        // ACT
        let iso = build_iso(&spec).expect("build_iso must succeed");

        // ASSERT
        let pvd_offset = SECTOR_SIZE * 16 + 1;
        assert_eq!(&iso[pvd_offset..pvd_offset + 5], b"CD001");
    }

    #[test]
    fn build_iso_aarch64_produces_valid_iso() {
        // ARRANGE
        let spec = BootFsSpec {
            boot_filename: Arch::Aarch64.boot_filename().to_owned(),
            uki: vec![0xCCu8; 512],
            files: vec![],
        };

        // ACT
        let iso = build_iso(&spec).expect("build_iso must succeed for aarch64");

        // ASSERT
        let pvd_offset = SECTOR_SIZE * 16 + 1;
        assert_eq!(&iso[pvd_offset..pvd_offset + 5], b"CD001");
    }

    #[test]
    fn build_img_returns_nonempty_image() {
        // ARRANGE
        let spec = BootFsSpec {
            boot_filename: Arch::Aarch64.boot_filename().to_owned(),
            uki: vec![0xABu8; 1024],
            files: vec![],
        };

        // ACT
        let img = build_img(&spec).expect("build_img must succeed");

        // ASSERT
        assert!(!img.is_empty());
    }

    #[test]
    fn build_img_with_extra_files_succeeds() {
        // ARRANGE
        let spec = BootFsSpec {
            boot_filename: Arch::Aarch64.boot_filename().to_owned(),
            uki: vec![0xCCu8; 512],
            files: vec![FileEntry {
                path: "config.txt".to_owned(),
                data: b"arm_64bit=1\n".to_vec(),
            }],
        };

        // ACT
        let img = build_img(&spec).expect("build_img must succeed with extra files");

        // ASSERT
        assert!(!img.is_empty());
    }

    #[test]
    fn build_iso_with_recursive_files_produces_valid_image() {
        // ARRANGE
        let spec = BootFsSpec {
            boot_filename: Arch::X86_64.boot_filename().to_owned(),
            uki: vec![0u8; 512],
            files: vec![FileEntry {
                path: "overlays/rpi/config.txt".to_owned(),
                data: b"arm_64bit=1".to_vec(),
            }],
        };

        // ACT
        let iso = build_iso(&spec).expect("build_iso must succeed");

        // ASSERT
        let pvd_offset = SECTOR_SIZE * 16 + 1;
        assert_eq!(&iso[pvd_offset..pvd_offset + 5], b"CD001");
    }
}
