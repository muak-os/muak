//! Kernel PE image parsing and validation.

use core::str;

use anyhow::{Context as _, Result, bail, ensure};
use object::LittleEndian as LE;
use object::pe::{
    IMAGE_DIRECTORY_ENTRY_BASERELOC, ImageDosHeader, ImageNtHeaders64, ImageSectionHeader,
};
use object::read::pe::{ImageNtHeaders as _, SectionTable};

/// `IMAGE_DLLCHARACTERISTICS_NX_COMPAT`.
const NX_COMPAT: u16 = 0x0100;

/// Minimum `MajorImageVersion` indicates `LINUX_INITRD_MEDIA_GUID` support.
const MIN_IMAGE_VERSION: u16 = 1;

/// Parsed inner kernel PE metadata.
#[derive(Debug)]
pub struct Image<'a> {
    pub data: &'a [u8],
    pub entry_point_rva: u32,
    pub base_address: u64,
    pub size: u32,
    pub nx_compat: bool,
    pub sections: SectionTable<'a>,
}

impl<'a> Image<'a> {
    /// Validates and extracts metadata from a kernel PE image.
    ///
    /// # Errors
    ///
    /// Returns an error if the kernel PE headers are invalid or contain unsupported relocation
    /// metadata.
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let dos_header = ImageDosHeader::parse(data).context("invalid DOS header")?;
        let mut offset = dos_header.nt_headers_offset().into();
        let (nt_headers, data_dirs) =
            ImageNtHeaders64::parse(data, &mut offset).context("invalid PE headers")?;
        let sections = nt_headers
            .sections(data, offset)
            .context("invalid section table")?;

        let optional_header = &nt_headers.optional_header;
        let entry_point_rva = optional_header.address_of_entry_point.get(LE);
        let base_address = optional_header.image_base.get(LE);
        let size = optional_header.size_of_image.get(LE);
        let major_version = optional_header.major_image_version.get(LE);
        let dll_chars = optional_header.dll_characteristics.get(LE);

        ensure!(entry_point_rva != 0, "kernel PE has no entry point");
        ensure!(
            major_version >= MIN_IMAGE_VERSION,
            "kernel PE MajorImageVersion={major_version}, need >= {MIN_IMAGE_VERSION} \
             (LINUX_INITRD_MEDIA_GUID support required)"
        );

        let reloc_size = data_dirs
            .get(IMAGE_DIRECTORY_ENTRY_BASERELOC)
            .map_or(0, |reloc_dir| reloc_dir.size.get(LE));
        ensure!(
            reloc_size == 0,
            "kernel PE has base relocations (size={reloc_size}), not supported"
        );

        if let Some(section) = sections
            .iter()
            .find(|section| section.pointer_to_relocations.get(LE) != 0)
        {
            let name = section_name(section);
            let ptr_relocs = section.pointer_to_relocations.get(LE);
            bail!("section {name} has relocations (pointer_to_relocations=0x{ptr_relocs:x})");
        }

        Ok(Image {
            data,
            entry_point_rva,
            base_address,
            size,
            nx_compat: dll_chars & NX_COMPAT != 0,
            sections,
        })
    }
}

/// Returns the section name as a string for diagnostics.
#[must_use]
pub fn section_name(section: &ImageSectionHeader) -> &str {
    str::from_utf8(&section.name)
        .unwrap_or("???")
        .trim_end_matches('\0')
}

#[cfg(test)]
mod tests {
    use object::pe::ImageSectionHeader;

    use super::*;
    use crate::pe::fixtures::{Builder, FILE_ALIGN_U32};

    #[test]
    fn parse_invalid_dos_header() {
        // ARRANGE
        let data = [0_u8; 256];

        // ACT
        let err = Image::parse(&data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("invalid DOS header"), "{err}");
    }

    #[test]
    fn parse_invalid_nt_headers() {
        // ARRANGE
        let mut data = vec![0_u8; 256];
        data.get_mut(..2)
            .expect("DOS magic range exists")
            .copy_from_slice(b"MZ");
        *data.get_mut(0x3C).expect("e_lfanew byte exists") = 0x40;

        // ACT
        let err = Image::parse(&data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("invalid PE headers"), "{err}");
    }

    #[test]
    fn parse_no_entry_point() {
        // ARRANGE
        let mut builder = Builder::new();
        builder.add_section(*b".text\0\0\0", &[0_u8; 16]);
        builder.set_entry_point(0);
        let data = builder.build();

        // ACT
        let err = Image::parse(&data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("no entry point"), "{err}");
    }

    #[test]
    fn parse_version_too_low() {
        // ARRANGE
        let mut builder = Builder::new();
        builder.add_section(*b".text\0\0\0", &[0_u8; 16]);
        builder.set_major_image_version(0);
        let data = builder.build();

        // ACT
        let err = Image::parse(&data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("MajorImageVersion"), "{err}");
    }

    #[test]
    fn parse_base_relocs_rejected() {
        // ARRANGE
        let mut builder = Builder::new();
        builder.add_section(*b".text\0\0\0", &[0_u8; 16]);
        builder.set_base_reloc_dir(0x1000, 64);
        let data = builder.build();

        // ACT
        let err = Image::parse(&data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("base relocations"), "{err}");
    }

    #[test]
    fn parse_base_reloc_dir_zero_size_ok() {
        // ARRANGE
        let mut builder = Builder::new();
        builder.add_section(*b".text\0\0\0", &[0_u8; 16]);
        builder.set_base_reloc_dir(0x1000, 0);
        let data = builder.build();

        // ACT + ASSERT
        Image::parse(&data).expect("zero-size base reloc dir should be allowed");
    }

    #[test]
    fn parse_section_relocations_rejected() {
        // ARRANGE
        let mut builder = Builder::new();
        builder.add_section(*b".text\0\0\0", &[0_u8; 16]);
        builder.set_last_ptr_relocs(0x500);
        let data = builder.build();

        // ACT
        let err = Image::parse(&data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("has relocations"), "{err}");
    }

    #[test]
    fn parse_nx_compat_true() {
        // ARRANGE
        let mut builder = Builder::new();
        builder.add_section(*b".text\0\0\0", &[0_u8; 16]);
        builder.set_dll_characteristics(0x0100);
        let data = builder.build();

        // ACT
        let kernel = Image::parse(&data).expect("should parse");

        // ASSERT
        assert!(kernel.nx_compat);
    }

    #[test]
    fn parse_nx_compat_false() {
        // ARRANGE
        let mut builder = Builder::new();
        builder.add_section(*b".text\0\0\0", &[0_u8; 16]);
        builder.set_dll_characteristics(0x0000);
        let data = builder.build();

        // ACT
        let kernel = Image::parse(&data).expect("should parse");

        // ASSERT
        assert!(!kernel.nx_compat);
    }

    #[test]
    fn parse_happy_path_metadata() {
        // ARRANGE
        let mut builder = Builder::new();
        builder.add_section(*b".text\0\0\0", &[0_u8; 16]);
        let data = builder.build();

        // ACT
        let kernel = Image::parse(&data).expect("should parse");

        // ASSERT
        assert_eq!(kernel.entry_point_rva, FILE_ALIGN_U32);
        assert_eq!(kernel.base_address, 0x0000_0000_0400_0000);
        assert!(!kernel.nx_compat);
    }

    #[test]
    fn section_name_valid_utf8_with_nul_padding() {
        // ARRANGE
        let header = ImageSectionHeader {
            name: *b".text\0\0\0",
            ..ImageSectionHeader::default()
        };

        // ACT + ASSERT
        assert_eq!(section_name(&header), ".text");
    }

    #[test]
    fn section_name_valid_utf8_no_padding() {
        // ARRANGE
        let header = ImageSectionHeader {
            name: *b".cmdline",
            ..ImageSectionHeader::default()
        };

        // ACT + ASSERT
        assert_eq!(section_name(&header), ".cmdline");
    }

    #[test]
    fn section_name_invalid_utf8_returns_fallback() {
        // ARRANGE
        let header = ImageSectionHeader {
            name: [0xFF, 0xFE, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
            ..ImageSectionHeader::default()
        };

        // ACT + ASSERT
        assert_eq!(section_name(&header), "???");
    }
}
