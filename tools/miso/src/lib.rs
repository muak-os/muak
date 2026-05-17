//! Miso - Packages a Unified Kernel Image into a bootable image.
#![deny(missing_docs)]

mod error;
mod iso;
mod raw;

use std::io::{Cursor, Write};

pub use error::MisoError;
pub use esp::{Arch, EspFile, EspSpec};
pub use iso::SECTOR_SIZE;

/// Builds a bootable ISO 9660 image from an `EspSpec` into any `Write + Seek` sink.
pub fn build_iso(
    spec: &EspSpec,
    out: &mut (impl std::io::Write + std::io::Seek),
) -> Result<(), MisoError> {
    let efi_image = esp::build(spec)?;
    iso::write(out, &efi_image)
}

/// Builds a raw GPT disk image from an `EspSpec` into any `Read + Write + Seek` sink.
pub fn build_raw(
    spec: &EspSpec,
    out: &mut impl Write,
    compression_level: Option<i32>,
) -> Result<(), MisoError> {
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

fn validate_compression_level(level: i32) -> Result<i32, MisoError> {
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
    use parttable::Table;

    use super::*;

    fn build_iso_bytes(spec: &EspSpec) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        build_iso(spec, &mut out).expect("build_iso must succeed");
        out.into_inner()
    }

    fn build_raw_bytes(spec: &EspSpec) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        build_raw(spec, &mut out, None).expect("build_raw must succeed");
        out.into_inner()
    }

    fn build_compressed_raw_bytes(spec: &EspSpec, compression_level: i32) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        build_raw(spec, &mut out, Some(compression_level))
            .expect("compressed build_raw must succeed");
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
        let spec = EspSpec::with_uki(Arch::X86_64, uki.clone(), vec![]);

        // ASSERT
        assert_eq!(spec.files.len(), 1);
        assert_eq!(spec.files[0].path, "EFI/BOOT/BOOTX64.EFI");
        assert_eq!(spec.files[0].data, uki);
    }

    #[test]
    fn with_uki_appends_extra_files_after_uki() {
        // ARRANGE
        let uki = vec![0u8; 32];
        let extra = EspFile {
            path: "config.txt".to_owned(),
            data: b"arm_64bit=1".to_vec(),
        };

        // ACT
        let spec = EspSpec::with_uki(Arch::Aarch64, uki, vec![extra.clone()]);

        // ASSERT
        assert_eq!(spec.files.len(), 2);
        assert_eq!(spec.files[0].path, "EFI/BOOT/BOOTAA64.EFI");
        assert_eq!(spec.files[1], extra);
    }

    #[test]
    fn build_iso_returns_nonempty_image() {
        // ARRANGE
        let spec = EspSpec::with_uki(Arch::X86_64, vec![0xABu8; 1024], vec![]);

        // ACT
        let iso = build_iso_bytes(&spec);

        // ASSERT
        assert!(!iso.is_empty());
    }

    #[test]
    fn build_iso_output_has_cd001_magic() {
        // ARRANGE
        let spec = EspSpec::with_uki(Arch::X86_64, vec![0u8; 512], vec![]);

        // ACT
        let iso = build_iso_bytes(&spec);

        // ASSERT
        let pvd_offset = SECTOR_SIZE * 16 + 1;
        assert_eq!(&iso[pvd_offset..pvd_offset + 5], b"CD001");
    }

    #[test]
    fn build_iso_aarch64_produces_valid_iso() {
        // ARRANGE
        let spec = EspSpec::with_uki(Arch::Aarch64, vec![0xCCu8; 512], vec![]);

        // ACT
        let iso = build_iso_bytes(&spec);

        // ASSERT
        let pvd_offset = SECTOR_SIZE * 16 + 1;
        assert_eq!(&iso[pvd_offset..pvd_offset + 5], b"CD001");
    }

    #[test]
    fn build_raw_returns_nonempty_image() {
        // ARRANGE
        let spec = EspSpec::with_uki(Arch::Aarch64, vec![0xABu8; 1024], vec![]);

        // ACT
        let img = build_raw_bytes(&spec);

        // ASSERT
        assert!(!img.is_empty());
    }

    #[test]
    fn build_raw_with_extra_files_succeeds() {
        // ARRANGE
        let spec = EspSpec::with_uki(
            Arch::Aarch64,
            vec![0xCCu8; 512],
            vec![EspFile {
                path: "config.txt".to_owned(),
                data: b"arm_64bit=1\n".to_vec(),
            }],
        );

        // ACT
        let img = build_raw_bytes(&spec);

        // ASSERT
        assert!(!img.is_empty());
    }

    #[test]
    fn build_raw_with_compression_produces_nonempty_output() {
        // ARRANGE
        let spec = EspSpec::with_uki(Arch::Aarch64, vec![0xABu8; 1024], vec![]);

        // ACT
        let compressed = build_compressed_raw_bytes(&spec, 3);

        // ASSERT
        assert!(!compressed.is_empty());
    }

    #[test]
    fn build_raw_with_compression_round_trips_to_valid_gpt() {
        // ARRANGE
        let spec = EspSpec::with_uki(Arch::Aarch64, vec![0xABu8; 1024], vec![]);

        // ACT
        let compressed = build_compressed_raw_bytes(&spec, 3);
        let raw = zstd::decode_all(&compressed[..]).expect("decode compressed raw");

        // ASSERT
        let mut cursor = Cursor::new(raw);
        let gpt = Table::read(&mut cursor).expect("image must contain a valid GPT");
        assert!(gpt.has_used_partitions());
    }

    #[test]
    fn build_raw_rejects_invalid_compression_level() {
        // ARRANGE
        let spec = EspSpec::with_uki(Arch::Aarch64, vec![0xABu8; 1024], vec![]);
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
            vec![0u8; 512],
            vec![EspFile {
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
