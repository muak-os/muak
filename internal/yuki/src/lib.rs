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
/// build_uki(
///     "stub.efi",
///     "kernel",
///     "initrd.img",
///     "cmdline.txt",
///     "uki.efi"
/// )?;
/// # Ok::<(), yuki::YukiError>(())
/// ```
pub fn build_uki(
    stub_path: impl AsRef<Path>,
    linux_path: impl AsRef<Path>,
    initrd_path: impl AsRef<Path>,
    cmdline_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
) -> Result<usize, YukiError> {
    let stub_path = stub_path.as_ref();
    let linux_path = linux_path.as_ref();
    let initrd_path = initrd_path.as_ref();
    let cmdline_path = cmdline_path.as_ref();
    let output_path = output_path.as_ref();

    let mut stub_data = Vec::new();
    fs::File::open(stub_path)
        .and_then(|mut f| f.read_to_end(&mut stub_data))
        .map_err(|e| YukiError::ReadError {
            file: stub_path.display().to_string(),
            source: e,
        })?;

    let linux_data = fs::read(linux_path).map_err(|e| YukiError::ReadError {
        file: linux_path.display().to_string(),
        source: e,
    })?;

    let initrd_data = fs::read(initrd_path).map_err(|e| YukiError::ReadError {
        file: initrd_path.display().to_string(),
        source: e,
    })?;

    let cmdline_data = fs::read(cmdline_path).map_err(|e| YukiError::ReadError {
        file: cmdline_path.display().to_string(),
        source: e,
    })?;

    let original_stub_len = stub_data.len();

    let (
        file_header_offset,
        optional_header_offset,
        section_table_offset,
        section_alignment,
        file_alignment,
        last_section_file_end,
        last_section_virtual_end,
        current_section_count,
    ) = {
        let pe = PeFile64::parse(&stub_data[..])
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
        let optional_header_size =
            nt_headers.file_header().size_of_optional_header.get(LE) as usize;
        let section_table_offset = optional_header_offset + optional_header_size;

        let section_alignment = read_u32(
            &stub_data,
            optional_header_offset + config::OPT_HEADER_SECTION_ALIGNMENT,
        );
        let file_alignment = read_u32(
            &stub_data,
            optional_header_offset + config::OPT_HEADER_FILE_ALIGNMENT,
        );

        let last_section_file_end = sections
            .iter()
            .map(|s| s.pointer_to_raw_data.get(LE) + s.size_of_raw_data.get(LE))
            .max()
            .unwrap_or(0);

        let last_section_virtual_end = sections
            .iter()
            .map(|s| {
                s.virtual_address.get(LE) + align_to(s.virtual_size.get(LE), section_alignment)
            })
            .max()
            .unwrap_or(0);

        let current_section_count = nt_headers.file_header().number_of_sections.get(LE);

        (
            file_header_offset,
            optional_header_offset,
            section_table_offset,
            section_alignment,
            file_alignment,
            last_section_file_end,
            last_section_virtual_end,
            current_section_count,
        )
    };

    let sections_to_add: [(&str, &[u8]); 4] = [
        (".cmdline", &cmdline_data),
        (".linux", &linux_data),
        (".initrd", &initrd_data),
        (".stub", &[]),
    ];

    let mut new_sections: Vec<(ImageSectionHeader, usize, usize)> = Vec::new();
    let mut current_file_offset = align_to(last_section_file_end, file_alignment);
    let mut current_virtual_address = align_to(last_section_virtual_end, section_alignment);

    let mut max_virtual_end = last_section_virtual_end;

    for (name, data) in &sections_to_add {
        let is_stub_section = *name == ".stub";
        let data_len = if is_stub_section {
            original_stub_len
        } else {
            data.len()
        };
        let virtual_size = data_len as u32;
        let size_of_raw_data = align_to(virtual_size, file_alignment);

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
            .max(current_virtual_address + align_to(virtual_size, section_alignment));

        new_sections.push((section, current_file_offset as usize, data_len));
        current_file_offset += size_of_raw_data;
        current_virtual_address += align_to(virtual_size, section_alignment);
    }

    if current_section_count as usize + sections_to_add.len() > u16::MAX as usize {
        return Err(YukiError::TooManySections);
    }

    let new_section_count = current_section_count + sections_to_add.len() as u16;
    let section_count_offset = file_header_offset + config::COFF_NUMBER_OF_SECTIONS;

    stub_data.resize(current_file_offset as usize, 0);

    stub_data[section_count_offset..section_count_offset + 2]
        .copy_from_slice(&new_section_count.to_le_bytes());

    for (i, (section_header, _, _)) in new_sections.iter().enumerate() {
        let offset = section_table_offset
            + (current_section_count as usize + i) * mem::size_of::<ImageSectionHeader>();
        let header_bytes = unsafe {
            std::slice::from_raw_parts(
                section_header as *const _ as *const u8,
                mem::size_of::<ImageSectionHeader>(),
            )
        };
        stub_data[offset..offset + header_bytes.len()].copy_from_slice(header_bytes);
    }

    for (i, (_, file_offset, data_len)) in new_sections.iter().enumerate() {
        let (name, data) = sections_to_add[i];
        if name == ".stub" {
            stub_data.copy_within(0..original_stub_len, *file_offset);
        } else {
            stub_data[*file_offset..*file_offset + *data_len].copy_from_slice(data);
        }
    }

    let size_of_image_off = optional_header_offset + config::OPT_HEADER_SIZE_OF_IMAGE;
    let new_size_of_image = align_to(max_virtual_end, section_alignment);
    write_u32(&mut stub_data, size_of_image_off, new_size_of_image);

    let output_len = stub_data.len();
    fs::write(output_path, &stub_data).map_err(|e| YukiError::WriteError {
        file: output_path.display().to_string(),
        source: e,
    })?;

    Ok(output_len)
}
