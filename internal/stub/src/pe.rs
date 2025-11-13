use uefi::Result;
use uefi::proto::loaded_image::LoadedImage;

#[derive(Debug)]
pub struct UkiSections {
    pub kernel: &'static [u8],
    pub cmdline: &'static [u8],
    pub initrd: &'static [u8],
    pub stub: Option<&'static [u8]>,
}

#[repr(C, packed)]
struct DosHeader {
    e_magic: [u8; 2],
    _reserved1: [u8; 58],
    e_lfanew: u32,
}

#[repr(C, packed)]
struct PeHeader {
    signature: [u8; 4],
    machine: u16,
    number_of_sections: u16,
    _reserved: [u8; 12],
    size_of_optional_header: u16,
    characteristics: u16,
}

#[repr(C, packed)]
struct SectionHeader {
    name: [u8; 8],
    virtual_size: u32,
    virtual_address: u32,
    size_of_raw_data: u32,
    pointer_to_raw_data: u32,
    _reserved: [u8; 12],
    characteristics: u32,
}

pub fn extract_sections(loaded_image: &LoadedImage) -> Result<UkiSections> {
    // Get the base address of our loaded image
    let base = loaded_image.info().0 as *const u8;
    let size = loaded_image.info().1 as usize;

    info!("Image base: {:p}, size: {} bytes", base, size);

    let image_data = unsafe { core::slice::from_raw_parts(base, size) };

    // Parse DOS header
    if image_data.len() < core::mem::size_of::<DosHeader>() {
        error!("Image too small for DOS header");
        return Err(uefi::Status::LOAD_ERROR.into());
    }

    let dos_header = unsafe { &*(image_data.as_ptr() as *const DosHeader) };

    // Verify DOS signature "MZ"
    if &dos_header.e_magic != b"MZ" {
        error!("Invalid DOS signature");
        return Err(uefi::Status::LOAD_ERROR.into());
    }

    let pe_offset = dos_header.e_lfanew as usize;

    // Parse PE header
    if image_data.len() < pe_offset + core::mem::size_of::<PeHeader>() {
        error!("Image too small for PE header");
        return Err(uefi::Status::LOAD_ERROR.into());
    }

    let pe_header = unsafe { &*((base as usize + pe_offset) as *const PeHeader) };

    // Verify PE signature "PE\0\0"
    if &pe_header.signature != b"PE\0\0" {
        error!("Invalid PE signature");
        return Err(uefi::Status::LOAD_ERROR.into());
    }

    let num_sections = pe_header.number_of_sections;
    info!("Found {} PE sections", num_sections);

    // Section headers start after PE header + optional header
    let section_offset =
        pe_offset + core::mem::size_of::<PeHeader>() + pe_header.size_of_optional_header as usize;

    // Find sections by name
    let mut kernel: Option<&'static [u8]> = None;
    let mut cmdline: Option<&'static [u8]> = None;
    let mut initrd: Option<&'static [u8]> = None;
    let mut stub: Option<&'static [u8]> = None;

    for i in 0..num_sections {
        let section_header_offset =
            section_offset + (i as usize * core::mem::size_of::<SectionHeader>());

        if image_data.len() < section_header_offset + core::mem::size_of::<SectionHeader>() {
            break;
        }

        let section =
            unsafe { &*((base as usize + section_header_offset) as *const SectionHeader) };

        // Get section name (null-terminated, max 8 bytes)
        let name_bytes = &section.name;
        let name_len = name_bytes.iter().position(|&b| b == 0).unwrap_or(8);
        let name = core::str::from_utf8(&name_bytes[..name_len]).unwrap_or("");

        // For loaded images, sections are at their virtual addresses
        // The virtual_address is an RVA (relative virtual address), but in the PE file
        // it's stored as an absolute address with a default image base (0x140000000 for UEFI)
        // We need to calculate the actual offset from our loaded image base
        const DEFAULT_IMAGE_BASE: usize = 0x140000000;
        let rva = if section.virtual_address as usize >= DEFAULT_IMAGE_BASE {
            section.virtual_address as usize - DEFAULT_IMAGE_BASE
        } else {
            section.virtual_address as usize
        };

        let section_start = rva;
        let section_size = section.virtual_size.min(section.size_of_raw_data) as usize;

        if section_start + section_size > size {
            warn!("Section {} extends beyond image, skipping", name);
            continue;
        }

        let section_data = &image_data[section_start..section_start + section_size];

        match name {
            ".linux" => {
                info!(
                    "Found .linux section: {} bytes at offset {:#x}",
                    section_size, section_start
                );
                kernel = Some(section_data);
            }
            ".cmdline" => {
                info!(
                    "Found .cmdline section: {} bytes at offset {:#x}",
                    section_size, section_start
                );
                cmdline = Some(section_data);
            }
            ".initrd" => {
                info!(
                    "Found .initrd section: {} bytes at offset {:#x}",
                    section_size, section_start
                );
                initrd = Some(section_data);
            }
            ".stub" => {
                info!(
                    "Found .stub section: {} bytes at offset {:#x}",
                    section_size, section_start
                );
                stub = Some(section_data);
            }
            _ => {}
        }
    }

    // Verify we found all required sections
    let kernel = kernel.ok_or_else(|| {
        error!(".linux section not found");
        uefi::Status::NOT_FOUND
    })?;

    let cmdline = cmdline.ok_or_else(|| {
        error!(".cmdline section not found");
        uefi::Status::NOT_FOUND
    })?;

    let initrd = initrd.ok_or_else(|| {
        error!(".initrd section not found");
        uefi::Status::NOT_FOUND
    })?;

    Ok(UkiSections {
        kernel,
        cmdline,
        initrd,
        stub,
    })
}
