//! Yuki - A library to create Unified Kernel Images (UKI) for Linux on UEFI systems.
//!
//! This library provides the core functionality for building UKIs by embedding
//! PE sections (cmdline, kernel, initrd, stub) into an EFI stub.

use std::fs;
use std::path::Path;
use std::result::Result;
use thiserror::Error;

mod binary;
mod config;
mod pe;
mod section;

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
/// # use std::path::Path;
/// yuki::build(
///     Path::new("stub.efi"),
///     Path::new("kernel"),
///     Path::new("initrd.img"),
///     Path::new("cmdline.txt"),
///     Path::new("uki.efi")
/// )?;
/// # Ok::<(), yuki::YukiError>(())
/// ```
pub fn build(
    stub_path: &Path,
    linux_path: &Path,
    initrd_path: &Path,
    cmdline_path: &Path,
    output_path: &Path,
) -> Result<usize, YukiError> {
    let stub = fs::read(stub_path).map_err(|e| YukiError::ReadError {
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

    let original_stub_len = stub.len();

    let metadata = pe::extract_metadata(&stub)?;

    if metadata.current_section_count as usize + 4 > u16::MAX as usize {
        return Err(YukiError::TooManySections);
    }

    let section_info =
        section::build_headers(&metadata, &linux, &initrd, &cmdline, original_stub_len)?;

    let mut stub_data = stub;
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

    section::write_to_image(
        &mut stub_data,
        &metadata,
        &section_info,
        &linux,
        &initrd,
        &cmdline,
        original_stub_len,
    )?;

    pe::update_image_size(&mut stub_data, &metadata, section_info.max_virtual_end);

    fs::write(output_path, &stub_data).map_err(|e| YukiError::WriteError {
        file: output_path.display().to_string(),
        source: e,
    })?;

    Ok(stub_data.len())
}
