//! PE parsing and validation for UKI stub & Kernel PE images

use anyhow::{Context, Result, anyhow, bail};
use object::LittleEndian as LE;
use object::pe::{
    IMAGE_DIRECTORY_ENTRY_BASERELOC, ImageDosHeader, ImageNtHeaders64, ImageSectionHeader,
};
use object::read::pe::{ImageNtHeaders, PeFile64, SectionTable};

/// `IMAGE_DLLCHARACTERISTICS_NX_COMPAT`
const NX_COMPAT: u16 = 0x0100;

/// Minimum `MajorImageVersion` — indicates `LINUX_INITRD_MEDIA_GUID` support
const MIN_IMAGE_VERSION: u16 = 1;

/// Parsed UKI sections from the PE image.
#[derive(Debug)]
pub struct UkiSections<'a> {
    pub linux: &'a [u8],
    pub initrd: Option<&'a [u8]>,
    pub cmdline: Option<&'a [u8]>,
    pub dtb: Option<&'a [u8]>,
    pub luks: Option<&'a [u8]>,
}

impl<'a> UkiSections<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        if data.len() < 0x40 {
            bail!("PE file too small (minimum 64 bytes required)");
        }
        let pe = PeFile64::parse(data).context("Failed to parse PE file")?;

        let sections = pe.section_table();

        let mut linux: Option<&'a [u8]> = None;
        let mut initrd: Option<&'a [u8]> = None;
        let mut cmdline: Option<&'a [u8]> = None;
        let mut dtb: Option<&'a [u8]> = None;
        let mut luks: Option<&'a [u8]> = None;

        for section in sections.iter() {
            let name = std::str::from_utf8(&section.name)
                .context("Invalid section name")?
                .trim_end_matches('\0');

            if !matches!(name, ".linux" | ".initrd" | ".cmdline" | ".dtb" | ".luks") {
                continue;
            }

            let rva = section.virtual_address.get(LE) as usize;
            let vs = section.virtual_size.get(LE) as usize;

            if vs == 0 {
                continue;
            }

            if rva + vs > data.len() {
                bail!(
                    "section {} data out of bounds: rva={:#x} size={:#x} data_len={:#x}",
                    name,
                    rva,
                    vs,
                    data.len()
                );
            }

            let section_data = &data[rva..rva + vs];

            match name {
                ".linux" => linux = Some(section_data),
                ".initrd" => initrd = Some(section_data),
                ".cmdline" => cmdline = Some(section_data),
                ".dtb" => dtb = Some(section_data),
                ".luks" => luks = Some(section_data),
                _ => unreachable!(),
            }
        }

        Ok(UkiSections {
            linux: linux.ok_or_else(|| anyhow!("UKI missing required .linux section"))?,
            initrd,
            cmdline,
            dtb,
            luks,
        })
    }

    /// Returns an iterator over sections to measure, in spec canonical order.
    pub fn iter_sections(&self) -> impl Iterator<Item = (&'static str, &'a [u8])> {
        [
            (".linux", Some(self.linux)),
            (".cmdline", self.cmdline),
            (".initrd", self.initrd),
            (".dtb", self.dtb),
        ]
        .into_iter()
        .filter_map(|(name, data)| data.map(|d| (name, d)))
    }
}

/// Parsed inner kernel PE metadata
#[derive(Debug)]
pub struct KernelPe<'a> {
    pub data: &'a [u8],
    pub entry_point_rva: u32,
    pub image_base: u64,
    pub size_of_image: u32,
    pub nx_compat: bool,
    pub sections: SectionTable<'a>,
}

impl<'a> KernelPe<'a> {
    /// Validates and extracts metadata from a kernel PE image
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let dos_header = ImageDosHeader::parse(data).context("invalid DOS header")?;
        let mut offset = dos_header.nt_headers_offset().into();
        let (nt_headers, data_dirs) =
            ImageNtHeaders64::parse(data, &mut offset).context("invalid PE headers")?;
        let sections = nt_headers
            .sections(data, offset)
            .context("invalid section table")?;

        let opt = &nt_headers.optional_header;
        let entry_point_rva = opt.address_of_entry_point.get(LE);
        let image_base = opt.image_base.get(LE);
        let size_of_image = opt.size_of_image.get(LE);
        let major_version = opt.major_image_version.get(LE);
        let dll_chars = opt.dll_characteristics.get(LE);

        if entry_point_rva == 0 {
            bail!("kernel PE has no entry point");
        }

        if major_version < MIN_IMAGE_VERSION {
            bail!(
                "kernel PE MajorImageVersion={major_version}, need >= {MIN_IMAGE_VERSION} \
                 (LINUX_INITRD_MEDIA_GUID support required)"
            );
        }

        if let Some(reloc_dir) = data_dirs.get(IMAGE_DIRECTORY_ENTRY_BASERELOC) {
            let size = reloc_dir.size.get(LE);
            if size != 0 {
                bail!("kernel PE has base relocations (size={size}), not supported");
            }
        }

        for section in sections.iter() {
            let ptr_relocs = section.pointer_to_relocations.get(LE);
            if ptr_relocs != 0 {
                let name = section_name(section);
                bail!("section {name} has relocations (pointer_to_relocations=0x{ptr_relocs:x})");
            }
        }

        Ok(KernelPe {
            data,
            entry_point_rva,
            image_base,
            size_of_image,
            nx_compat: dll_chars & NX_COMPAT != 0,
            sections,
        })
    }
}

/// Returns the section name as a string for diagnostics
pub fn section_name(section: &ImageSectionHeader) -> &str {
    std::str::from_utf8(&section.name)
        .unwrap_or("???")
        .trim_end_matches('\0')
}

#[cfg(test)]
mod tests {
    use super::*;

    // Layout (all little-endian):
    //
    //   0x000 – 0x03F  DOS header (64 bytes), e_lfanew = 0x40
    //   0x040 – 0x177  NT headers:
    //                    signature        4 B
    //                    file_header     20 B
    //                    optional_header 112 B (PE32+)
    //                    data_dirs       128 B (16 × 8 B)
    //   0x178 – 0x1FF  section headers (40 B each, up to 3 fit before 0x200)
    //   0x200 – …      section data (file_alignment = 0x200)

    const FILE_ALIGN: usize = 0x200;

    /// Returns the file offset of the NT headers (= 0x40).
    const NT_OFFSET: usize = 0x40;

    /// Offset of `optional_header` within NT headers.
    const OPT_OFF: usize = NT_OFFSET + 4 + 20; // signature + file_header

    /// Offset of the data directory array within the optional header.
    const DD_OFF: usize = OPT_OFF + 112;

    /// First section header starts here.
    const SHDR_OFF: usize = DD_OFF + 16 * 8;

    struct PeBuilder {
        data: Vec<u8>,
        num_sections: u16,
    }

    impl PeBuilder {
        /// Construct a minimal valid PE64 with no sections.
        fn new() -> Self {
            let hdr_size = FILE_ALIGN;
            let mut data = vec![0u8; hdr_size];

            data[0] = 0x4D; // 'M'
            data[1] = 0x5A; // 'Z'
            Self::write_u32(&mut data, 0x3C, NT_OFFSET as u32);

            data[NT_OFFSET] = b'P';
            data[NT_OFFSET + 1] = b'E';

            let fh = NT_OFFSET + 4;
            Self::write_u16(&mut data, fh, 0x8664); // AMD64
            // size_of_optional_header: 112 + 16*8 = 240 = 0xF0
            Self::write_u16(&mut data, fh + 16, 0xF0);
            Self::write_u16(&mut data, fh + 18, 0x0002); // executable

            Self::write_u16(&mut data, OPT_OFF, 0x020B); // magic PE32+
            // address_of_entry_point: first section lands at FILE_ALIGN
            Self::write_u32(&mut data, OPT_OFF + 16, FILE_ALIGN as u32);
            // image_base
            Self::write_u64(&mut data, OPT_OFF + 24, 0x0000_0000_0400_0000);
            // section_alignment == file_alignment so VAs equal file offsets
            Self::write_u32(&mut data, OPT_OFF + 32, FILE_ALIGN as u32);
            // file_alignment
            Self::write_u32(&mut data, OPT_OFF + 36, FILE_ALIGN as u32);
            // major_image_version at OPT_OFF+44: default 1
            Self::write_u16(&mut data, OPT_OFF + 44, 1);
            // size_of_image
            Self::write_u32(&mut data, OPT_OFF + 56, FILE_ALIGN as u32);
            // size_of_headers
            Self::write_u32(&mut data, OPT_OFF + 60, hdr_size as u32);
            // number_of_rva_and_sizes
            Self::write_u32(&mut data, OPT_OFF + 108, 16);

            Self {
                data,
                num_sections: 0,
            }
        }

        /// Append a section with `name` (max 8 bytes) and `content`.
        fn add_section(&mut self, name: &[u8; 8], content: &[u8]) -> u32 {
            let raw_offset = FILE_ALIGN * (self.num_sections as usize + 1);
            let raw_size = Self::align_up(content.len(), FILE_ALIGN);

            let needed = raw_offset + raw_size;
            if self.data.len() < needed {
                self.data.resize(needed, 0);
            }
            self.data[raw_offset..raw_offset + content.len()].copy_from_slice(content);

            let sh_off = SHDR_OFF + self.num_sections as usize * 40;
            self.data[sh_off..sh_off + 8].copy_from_slice(name);
            // virtual_size
            Self::write_u32(&mut self.data, sh_off + 8, content.len() as u32);
            // virtual_address == pointer_to_raw_data
            Self::write_u32(&mut self.data, sh_off + 12, raw_offset as u32);
            // size_of_raw_data
            Self::write_u32(&mut self.data, sh_off + 16, raw_size as u32);
            // pointer_to_raw_data
            Self::write_u32(&mut self.data, sh_off + 20, raw_offset as u32);

            self.num_sections += 1;

            let fh = NT_OFFSET + 4;
            Self::write_u16(&mut self.data, fh + 2, self.num_sections);

            let new_image_size = raw_offset + raw_size;
            Self::write_u32(&mut self.data, OPT_OFF + 56, new_image_size as u32);

            raw_offset as u32
        }

        /// Set the virtual_size of the last added section to `vs`.
        fn set_last_virtual_size(&mut self, vs: u32) {
            let sh_off = SHDR_OFF + (self.num_sections as usize - 1) * 40;
            Self::write_u32(&mut self.data, sh_off + 8, vs);
        }

        /// Set pointer_to_relocations on the last added section.
        fn set_last_ptr_relocs(&mut self, ptr: u32) {
            let sh_off = SHDR_OFF + (self.num_sections as usize - 1) * 40;
            Self::write_u32(&mut self.data, sh_off + 24, ptr);
        }

        /// Patch DLL characteristics in the optional header (offset 70 from opt start).
        fn set_dll_characteristics(&mut self, val: u16) {
            Self::write_u16(&mut self.data, OPT_OFF + 70, val);
        }

        /// Patch major_image_version (OPT_OFF+44).
        fn set_major_image_version(&mut self, val: u16) {
            Self::write_u16(&mut self.data, OPT_OFF + 44, val);
        }

        /// Patch address_of_entry_point.
        fn set_entry_point(&mut self, rva: u32) {
            Self::write_u32(&mut self.data, OPT_OFF + 16, rva);
        }

        /// Patch the base relocation data directory (index 5).
        fn set_base_reloc_dir(&mut self, vaddr: u32, size: u32) {
            let off = DD_OFF + IMAGE_DIRECTORY_ENTRY_BASERELOC * 8;
            Self::write_u32(&mut self.data, off, vaddr);
            Self::write_u32(&mut self.data, off + 4, size);
        }

        fn build(self) -> Vec<u8> {
            self.data
        }

        fn write_u16(buf: &mut [u8], off: usize, val: u16) {
            buf[off..off + 2].copy_from_slice(&val.to_le_bytes());
        }

        fn write_u32(buf: &mut [u8], off: usize, val: u32) {
            buf[off..off + 4].copy_from_slice(&val.to_le_bytes());
        }

        fn write_u64(buf: &mut [u8], off: usize, val: u64) {
            buf[off..off + 8].copy_from_slice(&val.to_le_bytes());
        }

        fn align_up(n: usize, align: usize) -> usize {
            (n + align - 1) & !(align - 1)
        }
    }

    #[test]
    fn uki_parse_too_small() {
        // ARRANGE

        let data = [0u8; 63];
        // ACT

        let err = UkiSections::parse(&data).unwrap_err();
        // ASSERT

        assert!(err.to_string().contains("too small"), "{err}");
    }

    #[test]
    fn uki_parse_invalid_pe() {
        // ARRANGE

        let data = [0u8; 256];
        // ACT

        let err = UkiSections::parse(&data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("Failed to parse PE file"), "{err}");
    }

    #[test]
    fn uki_parse_missing_linux_section() {
        // ARRANGE
        let mut b = PeBuilder::new();
        b.add_section(b".text\0\0\0", b"placeholder");
        let data = b.build();

        // ACT
        let err = UkiSections::parse(&data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("missing required .linux"), "{err}");
    }

    #[test]
    fn uki_parse_linux_only() {
        // ARRANGE
        let mut b = PeBuilder::new();
        b.add_section(b".linux\0\0", b"kernel_data");
        let data = b.build();

        // ACT
        let sects = UkiSections::parse(&data).expect("parse should succeed");

        // ASSERT
        assert_eq!(sects.linux, b"kernel_data");
        assert!(sects.initrd.is_none());
        assert!(sects.cmdline.is_none());
        assert!(sects.dtb.is_none());
        assert!(sects.luks.is_none());
    }

    #[test]
    fn uki_parse_all_sections() {
        // ARRANGE
        let mut b = PeBuilder::new();
        b.add_section(b".linux\0\0", b"linux");
        b.add_section(b".initrd\0", b"initrd");
        b.add_section(b".cmdline", b"cmdline");
        b.add_section(b".dtb\0\0\0\0", b"dtb");
        b.add_section(b".luks\0\0\0", b"luks");
        let data = b.build();

        // ACT
        let sects = UkiSections::parse(&data).expect("parse should succeed");

        // ASSERT
        assert_eq!(sects.linux, b"linux");
        assert_eq!(sects.initrd.expect("initrd"), b"initrd");
        assert_eq!(sects.cmdline.expect("cmdline"), b"cmdline");
        assert_eq!(sects.dtb.expect("dtb"), b"dtb");
        assert_eq!(sects.luks.expect("luks"), b"luks");
    }

    #[test]
    fn uki_parse_unrecognized_section_skipped() {
        // ARRANGE
        let mut b = PeBuilder::new();
        b.add_section(b".unknwn\0", b"ignored");
        b.add_section(b".linux\0\0", b"kernel");
        let data = b.build();

        // ACT
        let sects = UkiSections::parse(&data).expect("parse should succeed");

        // ASSERT
        assert_eq!(sects.linux, b"kernel");
    }

    #[test]
    fn uki_parse_zero_virtual_size_skipped() {
        // ARRANGE
        let mut b = PeBuilder::new();
        b.add_section(b".linux\0\0", b"kernel");
        b.add_section(b".initrd\0", b"initrd_data");
        b.set_last_virtual_size(0);
        let data = b.build();

        // ACT
        let sects = UkiSections::parse(&data).expect("parse should succeed");

        // ASSERT
        assert!(
            sects.initrd.is_none(),
            "zero-size section should be skipped"
        );
    }

    #[test]
    fn uki_parse_section_out_of_bounds() {
        // ARRANGE
        let mut b = PeBuilder::new();
        b.add_section(b".linux\0\0", b"kernel");
        b.set_last_virtual_size(0xFFFF_FF00);
        let data = b.build();

        // ACT
        let err = UkiSections::parse(&data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("out of bounds"), "{err}");
    }

    #[test]
    fn iter_sections_linux_only() {
        // ARRANGE
        let sects = UkiSections {
            linux: b"kern",
            initrd: None,
            cmdline: None,
            dtb: None,
            luks: None,
        };

        // ACT
        let items: Vec<_> = sects.iter_sections().collect();

        // ASSERT
        assert_eq!(items, vec![(".linux", b"kern" as &[u8])]);
    }

    #[test]
    fn iter_sections_all_present_excludes_luks() {
        // ARRANGE
        let sects = UkiSections {
            linux: b"kern",
            initrd: Some(b"initrd"),
            cmdline: Some(b"quiet"),
            dtb: Some(b"dtb"),
            luks: Some(b"secret"),
        };

        // ACT
        let items: Vec<_> = sects.iter_sections().collect();

        // ASSERT
        assert_eq!(
            items,
            vec![
                (".linux", b"kern" as &[u8]),
                (".cmdline", b"quiet" as &[u8]),
                (".initrd", b"initrd" as &[u8]),
                (".dtb", b"dtb" as &[u8]),
            ]
        );
        assert!(!items.iter().any(|(n, _)| *n == ".luks"));
    }

    #[test]
    fn iter_sections_canonical_order() {
        // ARRANGE
        let sects = UkiSections {
            linux: b"l",
            initrd: Some(b"i"),
            cmdline: Some(b"c"),
            dtb: None,
            luks: None,
        };

        // ACT
        let names: Vec<&str> = sects.iter_sections().map(|(n, _)| n).collect();

        // ASSERT
        assert_eq!(names, vec![".linux", ".cmdline", ".initrd"]);
    }

    #[test]
    fn kernel_parse_invalid_dos_header() {
        // ARRANGE
        let data = [0u8; 256];

        // ACT
        let err = KernelPe::parse(&data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("invalid DOS header"), "{err}");
    }

    #[test]
    fn kernel_parse_invalid_nt_headers() {
        // ARRANGE
        let mut data = vec![0u8; 256];
        data[0] = 0x4D;
        data[1] = 0x5A;
        data[0x3C] = 0x40;

        // ACT
        let err = KernelPe::parse(&data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("invalid PE headers"), "{err}");
    }

    #[test]
    fn kernel_parse_no_entry_point() {
        // ARRANGE
        let mut b = PeBuilder::new();
        b.add_section(b".text\0\0\0", &[0u8; 16]);
        b.set_entry_point(0);
        let data = b.build();

        // ACT
        let err = KernelPe::parse(&data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("no entry point"), "{err}");
    }

    #[test]
    fn kernel_parse_version_too_low() {
        // ARRANGE
        let mut b = PeBuilder::new();
        b.add_section(b".text\0\0\0", &[0u8; 16]);
        b.set_major_image_version(0);
        let data = b.build();

        // ACT
        let err = KernelPe::parse(&data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("MajorImageVersion"), "{err}");
    }

    #[test]
    fn kernel_parse_base_relocs_rejected() {
        // ARRANGE
        let mut b = PeBuilder::new();
        b.add_section(b".text\0\0\0", &[0u8; 16]);
        b.set_base_reloc_dir(0x1000, 64);
        let data = b.build();

        // ACT
        let err = KernelPe::parse(&data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("base relocations"), "{err}");
    }

    #[test]
    fn kernel_parse_base_reloc_dir_zero_size_ok() {
        // ARRANGE
        let mut b = PeBuilder::new();
        b.add_section(b".text\0\0\0", &[0u8; 16]);
        b.set_base_reloc_dir(0x1000, 0);
        let data = b.build();

        // ACT + ASSERT
        KernelPe::parse(&data).expect("zero-size base reloc dir should be allowed");
    }

    #[test]
    fn kernel_parse_section_relocations_rejected() {
        // ARRANGE
        let mut b = PeBuilder::new();
        b.add_section(b".text\0\0\0", &[0u8; 16]);
        b.set_last_ptr_relocs(0x500);
        let data = b.build();

        // ACT
        let err = KernelPe::parse(&data).unwrap_err();

        // ASSERT
        assert!(err.to_string().contains("has relocations"), "{err}");
    }

    #[test]
    fn kernel_parse_nx_compat_true() {
        // ARRANGE
        let mut b = PeBuilder::new();
        b.add_section(b".text\0\0\0", &[0u8; 16]);
        b.set_dll_characteristics(0x0100);
        let data = b.build();

        // ACT
        let k = KernelPe::parse(&data).expect("should parse");

        // ASSERT
        assert!(k.nx_compat);
    }

    #[test]
    fn kernel_parse_nx_compat_false() {
        // ARRANGE
        let mut b = PeBuilder::new();
        b.add_section(b".text\0\0\0", &[0u8; 16]);
        b.set_dll_characteristics(0x0000);
        let data = b.build();

        // ACT
        let k = KernelPe::parse(&data).expect("should parse");

        // ASSERT
        assert!(!k.nx_compat);
    }

    #[test]
    fn kernel_parse_happy_path_metadata() {
        // ARRANGE
        let mut b = PeBuilder::new();
        b.add_section(b".text\0\0\0", &[0u8; 16]);
        let data = b.build();

        // ACT
        let k = KernelPe::parse(&data).expect("should parse");

        // ASSERT
        assert_eq!(k.entry_point_rva, FILE_ALIGN as u32);
        assert_eq!(k.image_base, 0x0000_0000_0400_0000);
        assert!(!k.nx_compat);
    }

    #[test]
    fn section_name_valid_utf8_with_nul_padding() {
        // ARRANGE
        let mut hdr = ImageSectionHeader::default();
        hdr.name = *b".text\0\0\0";

        // ACT + ASSERT
        assert_eq!(section_name(&hdr), ".text");
    }

    #[test]
    fn section_name_valid_utf8_no_padding() {
        // ARRANGE
        let mut hdr = ImageSectionHeader::default();
        hdr.name = *b".cmdline";

        // ACT + ASSERT
        assert_eq!(section_name(&hdr), ".cmdline");
    }

    #[test]
    fn section_name_invalid_utf8_returns_fallback() {
        // ARRANGE
        let mut hdr = ImageSectionHeader::default();
        hdr.name = [0xFF, 0xFE, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

        // ACT + ASSERT
        assert_eq!(section_name(&hdr), "???");
    }
}
