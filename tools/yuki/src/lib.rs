//! Yuki - A library to create Unified Kernel Images (UKI) for Linux on UEFI systems.

#![warn(missing_docs)]

mod binary;
#[cfg(feature = "cli")]
pub mod cli;
pub mod error;
mod pe;
mod section;
mod stream;

use error::{Result, YukiError};

/// Borrowed component data required to build a Unified Kernel Image.
pub struct BuildInput<'a> {
    /// EFI stub PE binary to embed components into.
    pub stub: &'a [u8],
    /// Kernel image.
    pub kernel: &'a [u8],
    /// Initial RAM filesystem image.
    pub initramfs: &'a [u8],
    /// Kernel command-line string.
    pub cmdline: &'a [u8],
    /// Optional device-tree blob.
    pub dtb: Option<&'a [u8]>,
}

/// Builds a Unified Kernel Image (UKI) by embedding components into an EFI stub.
///
/// # Errors
///
/// Returns an error if the EFI stub is not a valid PE image or the resulting
/// image would exceed PE section limits, or writing the output fails.
pub fn build<W: std::io::Write>(input: &BuildInput<'_>, mut writer: W) -> Result<()> {
    let metadata = pe::extract_metadata(input.stub)?;

    let section_count = 3_u16.saturating_add(u16::from(input.dtb.is_some()));
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
    };

    let sections = section::build_section_list(&data);
    pe::validate_section_header_capacity(&metadata, sections.len())?;
    let layout = section::build_headers(&metadata, &sections)?;
    let new_section_count = metadata.current_section_count.saturating_add(section_count);

    stream::write(
        &mut writer,
        input.stub,
        &metadata,
        &layout,
        &sections,
        new_section_count,
    )
}
