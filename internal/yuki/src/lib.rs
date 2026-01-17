//! Yuki - A library to create Unified Kernel Images (UKI) for Linux on UEFI systems.
//!
//! This library provides the core functionality for building UKIs by embedding
//! PE sections (cmdline, kernel, initrd, stub) into an EFI stub.

use object::LittleEndian as LE;
use object::pe::{ImageFileHeader, ImageSectionHeader};
use object::read::pe::{ImageNtHeaders, PeFile64};
use std::fs;
use std::io::Read;
use std::mem;
use std::path::Path;
use std::result::Result;
use thiserror::Error;

pub mod binary;
pub mod config;

use binary::{align_to, read_u32, write_u32};

struct InputData {
    stub: Vec<u8>,
    linux: Vec<u8>,
    initrd: Vec<u8>,
    cmdline: Vec<u8>,
}

struct PeMetadata {
    file_header_offset: usize,
    optional_header_offset: usize,
    section_table_offset: usize,
    section_alignment: u32,
    file_alignment: u32,
    last_section_file_end: u32,
    last_section_virtual_end: u32,
    current_section_count: u16,
}

struct SectionInfo {
    headers: Vec<ImageSectionHeader>,
    offsets: Vec<(usize, usize)>,
    max_virtual_end: u32,
}

/// Error type for UKI building operations.
#[derive(Error, Debug)]
pub enum YukiError {
    #[error("Failed to read {file}: {source}")]
    ReadError {
        file: String,
        source: std::io::Error,
    },

    #[error("Failed to write {file}: {source}")]
    WriteError {
        file: String,
        source: std::io::Error,
    },

    #[error("Failed to parse PE file: {0}")]
    PeParseError(String),

    #[error("Invalid PE structure: {0}")]
    InvalidPeStructure(String),

    #[error("Too many sections: cannot add more sections to PE file")]
    TooManySections,
}

/// Builds a Unified Kernel Image (UKI) by embedding components into an EFI stub.
///
/// This function takes a PE format EFI stub and embeds the Linux kernel, initrd,
/// command line, and original stub data as PE sections to create a bootable UKI.
///
/// # Arguments
///
/// * `stub_path` - Path to the EFI stub file
/// * `linux_path` - Path to the Linux kernel image
/// * `initrd_path` - Path to the initrd image
/// * `cmdline_path` - Path to the kernel command line file
/// * `output_path` - Path where the UKI will be written
///
/// # Errors
///
/// Returns a `YukiError` if:
/// - Any input file cannot be read
/// - The stub file is not a valid PE file
/// - The output file cannot be written
/// - The PE structure is invalid
///
/// # Example
///
/// ```no_run
/// # use yuki::build_uki;
/// # use std::path::Path;
/// build_uki(
///     Path::new("stub.efi"),
///     Path::new("kernel"),
///     Path::new("initrd.img"),
///     Path::new("cmdline.txt"),
///     Path::new("uki.efi")
/// )?;
/// # Ok::<(), yuki::YukiError>(())
/// ```
pub fn build_uki(
    stub_path: &Path,
    linux_path: &Path,
    initrd_path: &Path,
    cmdline_path: &Path,
    output_path: &Path,
) -> Result<usize, YukiError> {
    let input_data = read_input_files(stub_path, linux_path, initrd_path, cmdline_path)?;
    let original_stub_len = input_data.stub.len();

    let metadata = extract_pe_metadata(&input_data.stub)?;

    if metadata.current_section_count as usize + 4 > u16::MAX as usize {
        return Err(YukiError::TooManySections);
    }

    let section_info = build_section_headers(
        &metadata,
        &input_data.linux,
        &input_data.initrd,
        &input_data.cmdline,
        original_stub_len,
    )?;

    let mut stub_data = input_data.stub;
    let new_file_size = section_info
        .offsets
        .last()
        .map(|(o, len)| o + len)
        .unwrap_or(0);
    stub_data.resize(new_file_size, 0);

    let new_section_count = metadata.current_section_count + 4u16;
    let section_count_offset = metadata.file_header_offset + config::COFF_NUMBER_OF_SECTIONS;
    stub_data[section_count_offset..section_count_offset + 2]
        .copy_from_slice(&new_section_count.to_le_bytes());

    write_sections_to_image(
        &mut stub_data,
        &metadata,
        &section_info,
        &input_data.linux,
        &input_data.initrd,
        &input_data.cmdline,
        original_stub_len,
    )?;

    update_pe_image_size(&mut stub_data, &metadata, section_info.max_virtual_end);

    fs::write(output_path, &stub_data).map_err(|e| YukiError::WriteError {
        file: output_path.display().to_string(),
        source: e,
    })?;

    Ok(stub_data.len())
}

fn read_input_files(
    stub_path: &Path,
    linux_path: &Path,
    initrd_path: &Path,
    cmdline_path: &Path,
) -> Result<InputData, YukiError> {
    let mut stub = Vec::new();
    fs::File::open(stub_path)
        .and_then(|mut f| f.read_to_end(&mut stub))
        .map_err(|e| YukiError::ReadError {
            file: stub_path.display().to_string(),
            source: e,
        })?;

    let linux = fs::read(linux_path).map_err(|e| YukiError::ReadError {
        file: linux_path.display().to_string(),
        source: e,
    })?;

    let initrd = fs::read(initrd_path).map_err(|e| YukiError::ReadError {
        file: initrd_path.display().to_string(),
        source: e,
    })?;

    let cmdline = fs::read(cmdline_path).map_err(|e| YukiError::ReadError {
        file: cmdline_path.display().to_string(),
        source: e,
    })?;

    Ok(InputData {
        stub,
        linux,
        initrd,
        cmdline,
    })
}

fn extract_pe_metadata(stub_data: &[u8]) -> Result<PeMetadata, YukiError> {
    let pe = PeFile64::parse(stub_data)
        .map_err(|_| YukiError::PeParseError("Invalid PE file format".to_string()))?;
    let nt_headers = pe.nt_headers();
    let sections = pe.section_table();

    let pe_offset = u32::from_le_bytes([
        stub_data[config::DOS_HEADER_PE_OFFSET],
        stub_data[config::DOS_HEADER_PE_OFFSET + 1],
        stub_data[config::DOS_HEADER_PE_OFFSET + 2],
        stub_data[config::DOS_HEADER_PE_OFFSET + 3],
    ]) as usize;
    let file_header_offset = pe_offset + config::PE_SIGNATURE_SIZE;
    let optional_header_offset = file_header_offset + mem::size_of::<ImageFileHeader>();
    let optional_header_size = nt_headers.file_header().size_of_optional_header.get(LE) as usize;
    let section_table_offset = optional_header_offset + optional_header_size;

    let section_alignment = read_u32(
        stub_data,
        optional_header_offset + config::OPT_HEADER_SECTION_ALIGNMENT,
    );
    let file_alignment = read_u32(
        stub_data,
        optional_header_offset + config::OPT_HEADER_FILE_ALIGNMENT,
    );

    let last_section_file_end = sections
        .iter()
        .map(|s| s.pointer_to_raw_data.get(LE) + s.size_of_raw_data.get(LE))
        .max()
        .unwrap_or(0);

    let last_section_virtual_end = sections
        .iter()
        .map(|s| s.virtual_address.get(LE) + align_to(s.virtual_size.get(LE), section_alignment))
        .max()
        .unwrap_or(0);

    let current_section_count = nt_headers.file_header().number_of_sections.get(LE);

    Ok(PeMetadata {
        file_header_offset,
        optional_header_offset,
        section_table_offset,
        section_alignment,
        file_alignment,
        last_section_file_end,
        last_section_virtual_end,
        current_section_count,
    })
}

fn build_section_headers(
    metadata: &PeMetadata,
    linux_data: &[u8],
    initrd_data: &[u8],
    cmdline_data: &[u8],
    original_stub_len: usize,
) -> Result<SectionInfo, YukiError> {
    let sections_to_add: [(&str, &[u8]); 4] = [
        (".cmdline", cmdline_data),
        (".linux", linux_data),
        (".initrd", initrd_data),
        (".stub", &[]),
    ];

    let mut headers = Vec::new();
    let mut offsets = Vec::new();
    let mut current_file_offset = align_to(metadata.last_section_file_end, metadata.file_alignment);
    let mut current_virtual_address = align_to(
        metadata.last_section_virtual_end,
        metadata.section_alignment,
    );
    let mut max_virtual_end = metadata.last_section_virtual_end;

    for (name, data) in &sections_to_add {
        let is_stub_section = *name == ".stub";
        let data_len = if is_stub_section {
            original_stub_len
        } else {
            data.len()
        };
        let virtual_size = data_len as u32;
        let size_of_raw_data = align_to(virtual_size, metadata.file_alignment);

        let mut section = ImageSectionHeader::default();

        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len().min(config::SECTION_NAME_MAX_LEN);
        section.name[..name_len].copy_from_slice(&name_bytes[..name_len]);

        section.virtual_size.set(LE, virtual_size);
        section.virtual_address.set(LE, current_virtual_address);
        section.size_of_raw_data.set(LE, size_of_raw_data);
        section.pointer_to_raw_data.set(LE, current_file_offset);

        let characteristics = if is_stub_section || *name == ".linux" {
            config::IMAGE_SCN_CNT_CODE | config::IMAGE_SCN_MEM_EXECUTE | config::IMAGE_SCN_MEM_READ
        } else {
            config::IMAGE_SCN_MEM_READ
        };
        section.characteristics.set(LE, characteristics);

        max_virtual_end = max_virtual_end
            .max(current_virtual_address + align_to(virtual_size, metadata.section_alignment));

        headers.push(section);
        offsets.push((current_file_offset as usize, data_len));
        current_file_offset += size_of_raw_data;
        current_virtual_address += align_to(virtual_size, metadata.section_alignment);
    }

    Ok(SectionInfo {
        headers,
        offsets,
        max_virtual_end,
    })
}

fn write_sections_to_image(
    stub_data: &mut [u8],
    metadata: &PeMetadata,
    section_info: &SectionInfo,
    linux_data: &[u8],
    initrd_data: &[u8],
    cmdline_data: &[u8],
    original_stub_len: usize,
) -> Result<(), YukiError> {
    let sections_to_add: [(&str, &[u8]); 4] = [
        (".cmdline", cmdline_data),
        (".linux", linux_data),
        (".initrd", initrd_data),
        (".stub", &[]),
    ];

    for (i, section_header) in section_info.headers.iter().enumerate() {
        let offset = metadata.section_table_offset
            + (metadata.current_section_count as usize + i) * mem::size_of::<ImageSectionHeader>();
        let header_bytes = section_header_to_bytes(section_header);
        let end = offset
            .checked_add(header_bytes.len())
            .ok_or(YukiError::InvalidPeStructure(
                "Section header offset overflow".to_string(),
            ))?;
        if end > stub_data.len() {
            return Err(YukiError::InvalidPeStructure(format!(
                "Section header offset out of bounds: {}-{}",
                offset, end
            )));
        }
        stub_data[offset..end].copy_from_slice(&header_bytes);
    }

    for (i, (file_offset, data_len)) in section_info.offsets.iter().enumerate() {
        let end = file_offset
            .checked_add(*data_len)
            .ok_or(YukiError::InvalidPeStructure(
                "Section data offset overflow".to_string(),
            ))?;
        if end > stub_data.len() {
            return Err(YukiError::InvalidPeStructure(format!(
                "Section data offset out of bounds: {}-{}",
                file_offset, end
            )));
        }
        let (name, _) = sections_to_add[i];
        if name == ".stub" {
            stub_data.copy_within(0..original_stub_len, *file_offset);
        } else {
            let data = sections_to_add[i].1;
            stub_data[*file_offset..end].copy_from_slice(data);
        }
    }

    Ok(())
}

fn section_header_to_bytes(
    header: &ImageSectionHeader,
) -> [u8; mem::size_of::<ImageSectionHeader>()] {
    let mut bytes = [0u8; mem::size_of::<ImageSectionHeader>()];

    bytes[0..8].copy_from_slice(&header.name);
    bytes[8..12].copy_from_slice(&header.virtual_size.get(LE).to_le_bytes());
    bytes[12..16].copy_from_slice(&header.virtual_address.get(LE).to_le_bytes());
    bytes[16..20].copy_from_slice(&header.size_of_raw_data.get(LE).to_le_bytes());
    bytes[20..24].copy_from_slice(&header.pointer_to_raw_data.get(LE).to_le_bytes());
    bytes[24..36].copy_from_slice(&[0u8; 12]);
    bytes[36..40].copy_from_slice(&header.characteristics.get(LE).to_le_bytes());

    bytes
}

fn update_pe_image_size(stub_data: &mut [u8], metadata: &PeMetadata, max_virtual_end: u32) {
    let size_of_image_off = metadata.optional_header_offset + config::OPT_HEADER_SIZE_OF_IMAGE;
    let new_size_of_image = align_to(max_virtual_end, metadata.section_alignment);
    write_u32(stub_data, size_of_image_off, new_size_of_image);
}
