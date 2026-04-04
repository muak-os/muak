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

/// Builds a bootable ISO 9660 image in memory containing the given UKI.
pub fn build_iso(uki: &[u8], arch: Arch) -> Result<Vec<u8>, MisoError> {
    let efi_image = fat::build_efi_image(uki, arch.boot_filename())?;
    let mut out = std::io::Cursor::new(Vec::new());
    iso::write_iso(&mut out, &efi_image)?;
    Ok(out.into_inner())
}

/// Builds a raw disk image with the given UKI and additional blobs.
pub fn build_img(uki: &[u8], blobs: &[(&str, &[u8])]) -> Result<Vec<u8>, MisoError> {
    let efi_image = fat::build_efi_image_with_blobs(uki, Arch::Aarch64.boot_filename(), blobs)?;
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
        let uki = vec![0xABu8; 1024];

        // ACT
        let iso = build_iso(&uki, Arch::X86_64).expect("build_iso must succeed");

        // ASSERT
        assert!(!iso.is_empty());
    }

    #[test]
    fn build_iso_output_has_cd001_magic() {
        // ARRANGE
        let uki = vec![0u8; 512];

        // ACT
        let iso = build_iso(&uki, Arch::X86_64).expect("build_iso must succeed");

        // ASSERT
        let pvd_offset = SECTOR_SIZE * 16 + 1;
        assert_eq!(&iso[pvd_offset..pvd_offset + 5], b"CD001");
    }

    #[test]
    fn build_iso_aarch64_produces_valid_iso() {
        // ARRANGE
        let uki = vec![0xCCu8; 512];

        // ACT
        let iso = build_iso(&uki, Arch::Aarch64).expect("build_iso must succeed for aarch64");

        // ASSERT
        let pvd_offset = SECTOR_SIZE * 16 + 1;
        assert_eq!(&iso[pvd_offset..pvd_offset + 5], b"CD001");
    }

    #[test]
    fn build_img_returns_nonempty_image() {
        // ARRANGE
        let uki = vec![0xABu8; 1024];

        // ACT
        let img = build_img(&uki, &[]).expect("build_img must succeed");

        // ASSERT
        assert!(!img.is_empty());
    }

    #[test]
    fn build_img_with_blobs_succeeds() {
        // ARRANGE
        let uki = vec![0xCCu8; 512];
        let blobs: &[(&str, &[u8])] = &[("config.txt", b"arm_64bit=1\n")];

        // ACT
        let img = build_img(&uki, blobs).expect("build_img must succeed with blobs");

        // ASSERT
        assert!(!img.is_empty());
    }
}
