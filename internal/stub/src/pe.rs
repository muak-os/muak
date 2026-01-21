use core::mem;
use core::slice;

use anyhow::{anyhow, Result};

const DOS_MAGIC: u16 = 0x5A4D; // Magic number MZ

/// DOS header - the legacy header at the start of PE files.
#[repr(C)]
pub struct ImageDosHeader {
    e_magic: u16,
    _unused: [u16; 29],
    e_lfanew: u32,
}

impl ImageDosHeader {
    pub fn validate(&self) -> Result<()> {
        if self.e_magic != DOS_MAGIC {
            return Err(anyhow!("invalid DOS header magic"));
        }
        Ok(())
    }

    pub fn pe_offset(&self) -> usize {
        self.e_lfanew as usize
    }
}

/// COFF file header - contains information about the PE file structure.
#[repr(C)]
pub struct ImageFileHeader {
    machine: u16,
    number_of_sections: u16,
    _unused: [u8; 12],
    size_of_optional_header: u16,
    _unused2: u16,
}

impl ImageFileHeader {
    pub fn section_count(&self) -> usize {
        self.number_of_sections as usize
    }

    pub fn optional_header_size(&self) -> usize {
        self.size_of_optional_header as usize
    }
}

/// Section header - describes a section in the PE file.
#[repr(C)]
pub struct ImageSectionHeader {
    name: [u8; 8],
    virtual_size: u32,
    virtual_address: u32,
    size_of_raw_data: u32,
    pointer_to_raw_data: u32,
    _unused: [u8; 12],
    characteristics: u32,
}

impl ImageSectionHeader {
    pub fn name_equals(&self, name: &[u8]) -> bool {
        for i in 0..8 {
            let c = self.name[i];
            let match_c = if i < name.len() { name[i] } else { 0 };
            if c != match_c {
                return false;
            }
        }
        true
    }

    pub fn virtual_address(&self) -> usize {
        self.virtual_address as usize
    }

    pub fn virtual_size(&self) -> usize {
        self.virtual_size as usize
    }
}

/// Parsed UKI sections from the PE image.
pub struct UkiSections<'a> {
    pub kernel: Option<&'a [u8]>,
    pub initrd: Option<&'a [u8]>,
    pub cmdline: Option<&'a [u8]>,
}

impl<'a> UkiSections<'a> {
    pub unsafe fn parse(base_addr: *const u8) -> Result<Self> {
        // SAFETY: caller guarantees base_addr points to valid PE image
        unsafe {
            let dos_header = &*(base_addr as *const ImageDosHeader);
            dos_header.validate()?;

            let pe_header_ptr = base_addr.add(dos_header.pe_offset());
            // Skip PE signature (4 bytes: "PE\0\0")
            let file_header_ptr = pe_header_ptr.add(4) as *const ImageFileHeader;
            let file_header = &*file_header_ptr;

            let section_headers_ptr = (file_header_ptr as *const u8)
                .add(mem::size_of::<ImageFileHeader>())
                .add(file_header.optional_header_size())
                as *const ImageSectionHeader;

            let sections = slice::from_raw_parts(section_headers_ptr, file_header.section_count());

            let mut result = UkiSections {
                kernel: None,
                initrd: None,
                cmdline: None,
            };

            for section in sections {
                let sec_start = base_addr.add(section.virtual_address());
                let sec_data = slice::from_raw_parts(sec_start, section.virtual_size());

                if section.name_equals(b".linux") {
                    result.kernel = Some(sec_data);
                } else if section.name_equals(b".initrd") {
                    result.initrd = Some(sec_data);
                } else if section.name_equals(b".cmdline") {
                    result.cmdline = Some(sec_data);
                }
            }

            Ok(result)
        }
    }

    pub fn require_kernel(&self) -> Result<&'a [u8]> {
        self.kernel
            .ok_or_else(|| anyhow!("no .linux section found"))
    }
}
