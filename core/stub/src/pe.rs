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
