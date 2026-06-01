//! Yuki - A library to create Unified Kernel Images (UKI) for Linux on UEFI systems.

mod binary;
#[cfg(feature = "cli")]
pub mod cli;
pub mod error;
mod pe;
mod section;

use error::{Result, YukiError};

/// Borrowed component data required to build a Unified Kernel Image.
pub struct BuildInput<'a> {
    pub stub: &'a [u8],
    pub kernel: &'a [u8],
    pub initramfs: &'a [u8],
    pub cmdline: &'a [u8],
    pub dtb: Option<&'a [u8]>,
    pub luks_key: Option<&'a [u8]>,
}

/// Builds a Unified Kernel Image (UKI) by embedding components into an EFI stub.
///
/// # Errors
///
/// Returns an error if the EFI stub is not a valid PE image or the resulting
/// image would exceed PE section limits.
pub fn build(input: &BuildInput<'_>) -> Result<Vec<u8>> {
    let mut stub = input.stub.to_vec();
    let metadata = pe::extract_metadata(&stub)?;

    let section_count = 3_u16
        .saturating_add(u16::from(input.dtb.is_some()))
        .saturating_add(u16::from(input.luks_key.is_some()));
    if usize::from(metadata.current_section_count).saturating_add(usize::from(section_count))
        > usize::from(u16::MAX)
    {
        return Err(YukiError::TooManySections);
    }

    let data = section::SectionData {
        linux: input.kernel,
        initrd: input.initramfs,
        cmdline: input.cmdline,
        dtb: input.dtb,
        luks: input.luks_key,
    };

    let sections = section::build_section_list(&data);
    pe::validate_section_header_capacity(&metadata, sections.len())?;
    let layout = section::build_headers(&metadata, &sections)?;

    stub.resize(layout.total_file_size, 0);

    let new_section_count = metadata
        .current_section_count
        .checked_add(section_count)
        .ok_or(YukiError::TooManySections)?;

    pe::update_section_count(&mut stub, &metadata, new_section_count)?;
    section::write_to_image(&mut stub, &metadata, &layout, &sections)?;
    pe::update_image_size(&mut stub, &metadata, layout.max_virtual_end)?;

    Ok(stub)
}
