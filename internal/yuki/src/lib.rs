//! Yuki - A library to create Unified Kernel Images (UKI) for Linux on UEFI systems.
//!
//! This library provides the core functionality for building UKIs by embedding
//! PE sections (cmdline, dtb, linux, initrd) into an EFI stub.

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
/// command line, optional device tree blob and optional LUKS key as PE sections
/// to create a bootable UKI.
///
/// # Arguments
///
/// * `stub_path` - Path to the EFI stub file
/// * `linux_path` - Path to the Linux kernel image
/// * `initrd_path` - Path to the initrd image
/// * `cmdline_path` - Path to the kernel command line file
/// * `dtb_path` - Optional path to a device tree blob (for ARM64 platforms)
/// * `luks_data` - Optional raw LUKS key bytes to embed as a `.luks` PE section
///
/// # Returns
///
/// The UKI buffer as a `Vec<u8>`.
///
/// # Errors
///
/// Returns a `YukiError` if:
/// - Any input file cannot be read
/// - The stub file is not a valid PE file
/// - The PE structure is invalid
///
/// # Example
///
/// ```no_run
/// # use std::path::Path;
/// let buffer = yuki::build(
///     Path::new("stub.efi"),
///     Path::new("kernel"),
///     Path::new("initrd.img"),
///     Path::new("cmdline.txt"),
///     None, // No DTB
///     None, // No LUKS key
/// )?;
/// # Ok::<(), yuki::YukiError>(())
/// ```
pub fn build(
    stub_path: &Path,
    linux_path: &Path,
    initrd_path: &Path,
    cmdline_path: &Path,
    dtb_path: Option<&Path>,
    luks_data: Option<&[u8]>,
) -> Result<Vec<u8>, YukiError> {
    let mut stub = fs::read(stub_path).map_err(|e| YukiError::ReadError {
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

    let dtb = dtb_path
        .map(|path| {
            fs::read(path).map_err(|e| YukiError::ReadError {
                file: path.display().to_string(),
                source: e,
            })
        })
        .transpose()?;

    let metadata = pe::extract_metadata(&stub)?;

    let mut section_count = 3;
    if dtb.is_some() {
        section_count += 1;
    }
    if luks_data.is_some() {
        section_count += 1;
    }
    if metadata.current_section_count as usize + section_count > u16::MAX as usize {
        return Err(YukiError::TooManySections);
    }

    let data = section::SectionData {
        linux: &linux,
        initrd: &initrd,
        cmdline: &cmdline,
        dtb: dtb.as_deref(),
        luks: luks_data,
    };

    let section_info = section::build_headers(&metadata, &data)?;

    stub.resize(section_info.total_file_size, 0);

    let new_section_count = metadata.current_section_count + section_count as u16;
    let section_count_offset = metadata.file_header_offset + config::COFF_NUMBER_OF_SECTIONS;
    stub[section_count_offset..section_count_offset + 2]
        .copy_from_slice(&new_section_count.to_le_bytes());

    section::write_to_image(&mut stub, &metadata, &section_info, &data)?;

    pe::update_image_size(&mut stub, &metadata, section_info.max_virtual_end);

    Ok(stub)
}
