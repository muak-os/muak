//! Miso - Packages a Unified Kernel Image into a bootable image.

#![warn(missing_docs)]

#[cfg(feature = "cli")]
pub mod cli;
pub mod error;
pub mod iso;
pub mod raw;

use std::io::{Cursor, Write};

use esp::EspSpec;

use crate::error::{MisoError, Result};

/// Builds a bootable ISO 9660 image from an `esp::EspSpec`.
///
/// # Errors
///
/// Returns an error if ESP construction fails or writing the ISO image fails.
pub fn build_iso<W: Write>(spec: &EspSpec, out: &mut W) -> Result<()> {
    let efi_image = esp::build(spec)?;
    iso::write(out, &efi_image)
}

/// Builds a raw GPT disk image from an `esp::EspSpec` into any `Read + Write + Seek` sink.
///
/// # Errors
///
/// Returns an error if ESP construction fails, compression level validation fails,
/// raw image creation fails, or output writing/compression fails.
pub fn build_raw<W: Write>(
    spec: &EspSpec,
    out: &mut W,
    compression_level: Option<i32>,
) -> Result<()> {
    let efi_image = esp::build(spec)?;
    let mut raw_out = Cursor::new(Vec::new());
    raw::write(&mut raw_out, &efi_image)?;
    let raw_bytes = raw_out.into_inner();

    if let Some(level) = compression_level {
        let level = validate_compression_level(level)?;
        let mut encoder = zstd::Encoder::new(out, level).map_err(MisoError::ZstdInit)?;
        encoder.write_all(&raw_bytes)?;
        encoder.finish().map_err(MisoError::Compression)?;
    } else {
        out.write_all(&raw_bytes)?;
    }

    Ok(())
}

fn validate_compression_level(level: i32) -> Result<i32> {
    let range = zstd::compression_level_range();

    if level == 0 || range.contains(&level) {
        Ok(level)
    } else {
        Err(MisoError::InvalidCompressionLevel {
            level,
            min: *range.start(),
            max: *range.end(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use esp::{Arch, EspFile};

    use super::*;

    fn build_iso_bytes(spec: &EspSpec) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        build_iso(spec, &mut out).expect("build_iso must succeed");
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
        let uki = vec![0xAB_u8; 64];

        // ACT
        let spec = EspSpec::with_uki(Arch::X86_64, uki.clone(), vec![]);

        // ASSERT
        assert_eq!(spec.files.len(), 1);
        let boot_file = spec.files.first().expect("boot file must exist");
        assert_eq!(boot_file.path, "EFI/BOOT/BOOTX64.EFI");
        assert_eq!(boot_file.data, uki);
    }

    #[test]
    fn with_uki_appends_extra_files_after_uki() {
        // ARRANGE
        let uki = vec![0_u8; 32];
        let extra = EspFile {
            path: "config.txt".to_owned(),
            data: b"arm_64bit=1".to_vec(),
        };

        // ACT
        let spec = EspSpec::with_uki(Arch::Aarch64, uki, vec![extra.clone()]);

        // ASSERT
        assert_eq!(spec.files.len(), 2);
        let boot_file = spec.files.first().expect("boot file must exist");
        let extra_file = spec.files.get(1).expect("extra file must exist");
        assert_eq!(boot_file.path, "EFI/BOOT/BOOTAA64.EFI");
        assert_eq!(extra_file, &extra);
    }

    #[test]
    fn build_iso_returns_nonempty_image() {
        // ARRANGE
        let spec = EspSpec::with_uki(Arch::X86_64, vec![0xAB_u8; 1024], vec![]);

        // ACT
        let iso = build_iso_bytes(&spec);

        // ASSERT
        assert!(!iso.is_empty());
    }

    #[test]
    fn build_iso_output_has_cd001_magic() {
        // ARRANGE
        let spec = EspSpec::with_uki(Arch::X86_64, vec![0_u8; 512], vec![]);

        // ACT
        let iso = build_iso_bytes(&spec);

        // ASSERT
        let pvd_offset = iso::SECTOR_SIZE * 16 + 1;
        assert_eq!(
            iso.get(pvd_offset..pvd_offset + 5)
                .expect("PVD magic must exist"),
            b"CD001"
        );
    }

    #[test]
    fn build_iso_aarch64_produces_valid_iso() {
        // ARRANGE
        let spec = EspSpec::with_uki(Arch::Aarch64, vec![0xCC_u8; 512], vec![]);

        // ACT
        let iso = build_iso_bytes(&spec);

        // ASSERT
        let pvd_offset = iso::SECTOR_SIZE * 16 + 1;
        assert_eq!(
            iso.get(pvd_offset..pvd_offset + 5)
                .expect("PVD magic must exist"),
            b"CD001"
        );
    }

    #[test]
    fn build_raw_rejects_invalid_compression_level() {
        // ARRANGE
        let spec = EspSpec::with_uki(Arch::Aarch64, vec![0xAB_u8; 1024], vec![]);
        let mut out = Cursor::new(Vec::new());

        // ACT
        let result = build_raw(&spec, &mut out, Some(i32::MAX));

        // ASSERT
        assert!(matches!(
            result,
            Err(MisoError::InvalidCompressionLevel { .. })
        ));
    }

    #[test]
    fn build_iso_with_recursive_files_produces_valid_image() {
        // ARRANGE
        let spec = EspSpec::with_uki(
            Arch::X86_64,
            vec![0_u8; 512],
            vec![EspFile {
                path: "overlays/rpi/config.txt".to_owned(),
                data: b"arm_64bit=1".to_vec(),
            }],
        );

        // ACT
        let iso = build_iso_bytes(&spec);

        // ASSERT
        let pvd_offset = iso::SECTOR_SIZE * 16 + 1;
        assert_eq!(
            iso.get(pvd_offset..pvd_offset + 5)
                .expect("PVD magic must exist"),
            b"CD001"
        );
    }
}
