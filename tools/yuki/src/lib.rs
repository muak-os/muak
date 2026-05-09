//! Yuki - A library to create Unified Kernel Images (UKI) for Linux on UEFI systems.

use std::fs;
use std::path::PathBuf;
use std::result::Result;

mod binary;
mod constants;
mod error;
mod pe;
mod section;

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
/// # Example
///
/// ```no_run
/// use std::path::PathBuf;
/// let buffer = yuki::build(&yuki::Components {
///     stub: PathBuf::from("stub.efi"),
///     kernel: PathBuf::from("vmlinuz"),
///     initramfs: PathBuf::from("initramfs.img"),
///     cmdline: PathBuf::from("cmdline.txt"),
///     dtb: None,
///     luks_key: None,
/// })?;
/// # Ok::<(), yuki::YukiError>(())
/// ```
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

    let sections = section::build_section_list(&data);
    let layout = section::build_headers(&metadata, &sections);

    stub.resize(layout.total_file_size, 0);

    let new_section_count = metadata.current_section_count + section_count as u16;
    let section_count_offset = metadata.file_header_offset + constants::COFF_NUMBER_OF_SECTIONS;
    stub[section_count_offset..section_count_offset + 2]
        .copy_from_slice(&new_section_count.to_le_bytes());

    section::write_to_image(&mut stub, &metadata, &layout, &sections)?;

    pe::update_image_size(&mut stub, &metadata, layout.max_virtual_end);

    Ok(stub)
}
