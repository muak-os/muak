//! Yuki - A library to create Unified Kernel Images (UKI) for Linux on UEFI systems.

mod binary;
#[cfg(feature = "cli")]
pub mod cli;
mod constants;
mod error;
mod pe;
mod section;

use std::fs;
use std::path::PathBuf;
use std::result::Result;

pub use error::YukiError;

/// Paths to the components required to build a Unified Kernel Image.
pub struct Components {
    pub stub: PathBuf,
    pub kernel: PathBuf,
    pub initramfs: PathBuf,
    pub cmdline: PathBuf,
    pub dtb: Option<PathBuf>,
    pub luks_key: Option<Vec<u8>>,
}

/// Builds a Unified Kernel Image (UKI) by embedding components into an EFI stub.
///
/// # Errors
///
/// Returns an error if any input component cannot be read, the EFI stub is not a
/// valid PE image, or the resulting image would exceed PE section limits.
pub fn build(c: &Components) -> Result<Vec<u8>, YukiError> {
    let mut stub = fs::read(&c.stub).map_err(|e| YukiError::ReadError {
        file: c.stub.display().to_string(),
        source: e,
    })?;

    let linux = fs::read(&c.kernel).map_err(|e| YukiError::ReadError {
        file: c.kernel.display().to_string(),
        source: e,
    })?;

    let initrd = fs::read(&c.initramfs).map_err(|e| YukiError::ReadError {
        file: c.initramfs.display().to_string(),
        source: e,
    })?;

    let cmdline = fs::read(&c.cmdline).map_err(|e| YukiError::ReadError {
        file: c.cmdline.display().to_string(),
        source: e,
    })?;

    let dtb = c
        .dtb
        .as_ref()
        .map(|path| {
            fs::read(path).map_err(|e| YukiError::ReadError {
                file: path.display().to_string(),
                source: e,
            })
        })
        .transpose()?;

    let luks_data = c.luks_key.as_deref();

    let metadata = pe::extract_metadata(&stub)?;

    let section_count = 3_u16
        .saturating_add(u16::from(dtb.is_some()))
        .saturating_add(u16::from(luks_data.is_some()));
    if usize::from(metadata.current_section_count).saturating_add(usize::from(section_count))
        > usize::from(u16::MAX)
    {
        return Err(YukiError::TooManySections);
    }

    let data = section::SectionData {
        linux: &linux,
        initrd: &initrd,
        cmdline: &cmdline,
        dtb: dtb.as_deref(),
        luks: luks_data,
    };

    let sections = section::build_section_list(&data);
    pe::validate_section_header_capacity(&metadata, sections.len())?;
    let layout = section::build_headers(&metadata, &sections)?;

    stub.resize(layout.total_file_size, 0);

    let new_section_count = metadata
        .current_section_count
        .checked_add(section_count)
        .ok_or(YukiError::TooManySections)?;
    let section_count_offset = metadata
        .file_header_offset
        .saturating_add(constants::COFF_NUMBER_OF_SECTIONS);
    binary::write_u16(&mut stub, section_count_offset, new_section_count)?;

    section::write_to_image(&mut stub, &metadata, &layout, &sections)?;

    pe::update_image_size(&mut stub, &metadata, layout.max_virtual_end)?;

    Ok(stub)
}
