//! PE file parsing and manipulation.

use object::LittleEndian as LE;
use object::pe::{ImageFileHeader, ImageSectionHeader};
use object::read::pe::{ImageNtHeaders as _, PeFile64};

use crate::binary;
use crate::error::{Result, YukiError};

/// Size of the PE signature in bytes ("PE\0\0").
const PE_SIGNATURE_SIZE: usize = 4;

/// Byte offset within the COFF file header to the `NumberOfSections` field.
const COFF_NUMBER_OF_SECTIONS_OFFSET: usize = 2;

/// Byte offset within the optional header to the `SizeOfImage` field.
const OPT_HEADER_SIZE_OF_IMAGE_OFFSET: usize = 56;

pub struct PeMetadata {
    pub file_header_offset: usize,
    pub optional_header_offset: usize,
    pub section_table_offset: usize,
    pub size_of_headers: u32,
    pub section_alignment: u32,
    pub file_alignment: u32,
    pub last_section_file_end: u32,
    pub last_section_virtual_end: u32,
    pub current_section_count: u16,
}

/// Extracts PE metadata from the given stub data, validating the structure and returning relevant offsets and alignment information.
pub fn extract_metadata(stub_data: &[u8]) -> Result<PeMetadata> {
    let pe = PeFile64::parse(stub_data)
        .map_err(|err| YukiError::PeParseError(format!("Invalid PE file format: {err}")))?;
    let nt_headers = pe.nt_headers();
    let sections = pe.section_table();

    let pe_offset = binary::usize_from_u128(
        u128::from(pe.dos_header().e_lfanew.get(LE)),
        "PE offset does not fit in usize",
    )?;
    let file_header_offset = pe_offset.saturating_add(PE_SIGNATURE_SIZE);
    let optional_header_offset =
        file_header_offset.saturating_add(core::mem::size_of::<ImageFileHeader>());
    let optional_header_size =
        usize::from(nt_headers.file_header().size_of_optional_header.get(LE));
    let section_table_offset = optional_header_offset.saturating_add(optional_header_size);

    let optional_header = nt_headers.optional_header();
    let section_alignment = optional_header.section_alignment.get(LE);
    let file_alignment = optional_header.file_alignment.get(LE);
    let size_of_headers = optional_header.size_of_headers.get(LE);

    if section_alignment == 0 || !section_alignment.is_power_of_two() {
        return Err(YukiError::InvalidPeStructure(format!(
            "invalid section alignment {section_alignment}"
        )));
    }

    if file_alignment == 0 || !file_alignment.is_power_of_two() {
        return Err(YukiError::InvalidPeStructure(format!(
            "invalid file alignment {file_alignment}"
        )));
    }

    if size_of_headers == 0 {
        return Err(YukiError::InvalidPeStructure(
            "invalid size of headers 0".to_owned(),
        ));
    }

    let (last_section_file_end, last_section_virtual_end) =
        sections
            .iter()
            .try_fold((0_u32, 0_u32), |(max_file, max_virt), section| {
                let file_end = section
                    .pointer_to_raw_data
                    .get(LE)
                    .checked_add(section.size_of_raw_data.get(LE))
                    .ok_or_else(|| {
                        YukiError::InvalidPeStructure("section raw data end overflow".to_owned())
                    })?;
                let aligned_virtual_size =
                    binary::align_to(section.virtual_size.get(LE), section_alignment);
                let virt_end = section
                    .virtual_address
                    .get(LE)
                    .checked_add(aligned_virtual_size)
                    .ok_or_else(|| {
                        YukiError::InvalidPeStructure("section virtual end overflow".to_owned())
                    })?;
                Ok::<(u32, u32), YukiError>((max_file.max(file_end), max_virt.max(virt_end)))
            })?;

    let current_section_count = nt_headers.file_header().number_of_sections.get(LE);

    Ok(PeMetadata {
        file_header_offset,
        optional_header_offset,
        section_table_offset,
        size_of_headers,
        section_alignment,
        file_alignment,
        last_section_file_end,
        last_section_virtual_end,
        current_section_count,
    })
}

/// Returns the byte offset of the `NumberOfSections` field in the PE COFF header.
#[must_use]
pub fn section_count_offset(metadata: &PeMetadata) -> usize {
    metadata
        .file_header_offset
        .saturating_add(COFF_NUMBER_OF_SECTIONS_OFFSET)
}

/// Returns the byte offset of the `SizeOfImage` field in the PE optional header.
#[must_use]
pub fn size_of_image_offset(metadata: &PeMetadata) -> usize {
    metadata
        .optional_header_offset
        .saturating_add(OPT_HEADER_SIZE_OF_IMAGE_OFFSET)
}

/// Validates that the section header table can accommodate the specified number of additional sections.
pub fn validate_section_header_capacity(
    metadata: &PeMetadata,
    additional_sections: usize,
) -> Result<()> {
    let total_sections =
        usize::from(metadata.current_section_count).saturating_add(additional_sections);
    let section_table_size =
        total_sections.saturating_mul(core::mem::size_of::<ImageSectionHeader>());
    let section_table_end = metadata
        .section_table_offset
        .saturating_add(section_table_size);
    let size_of_headers = binary::usize_from_u128(
        u128::from(metadata.size_of_headers),
        "size of headers does not fit in usize",
    )?;

    if section_table_end > size_of_headers {
        return Err(YukiError::InvalidPeStructure(format!(
            "section table exceeds size of headers: {section_table_end}>{size_of_headers}"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_bytes(bytes: &mut [u8], offset: usize, data: &[u8]) {
        let end = offset.saturating_add(data.len());
        bytes.get_mut(offset..end).unwrap().copy_from_slice(data);
    }

    #[test]
    fn pe_metadata_structure() {
        // ARRANGE
        let metadata = PeMetadata {
            file_header_offset: 64,
            optional_header_offset: 84,
            section_table_offset: 324,
            size_of_headers: 512,
            section_alignment: 4096,
            file_alignment: 512,
            last_section_file_end: 512,
            last_section_virtual_end: 4096,
            current_section_count: 1,
        };

        // ACT & ASSERT
        assert_eq!(metadata.file_header_offset, 64);
        assert_eq!(metadata.optional_header_offset, 84);
        assert_eq!(metadata.section_alignment, 4096);
        assert_eq!(metadata.current_section_count, 1);
        assert_eq!(metadata.section_table_offset, 324);
        assert_eq!(metadata.size_of_headers, 512);
        assert_eq!(metadata.file_alignment, 512);
        assert_eq!(metadata.last_section_file_end, 512);
        assert_eq!(metadata.last_section_virtual_end, 4096);
    }

    #[test]
    fn extract_metadata_invalid_pe() {
        // ARRANGE
        let stub = vec![0_u8; 100];

        // ACT
        let result = extract_metadata(&stub);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn extract_metadata_too_small() {
        // ARRANGE
        let stub = [0x4D, 0x5A];

        // ACT
        let result = extract_metadata(&stub);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn extract_metadata_with_malformed_header() {
        // ARRANGE
        let mut stub = vec![0_u8; 100];
        write_bytes(&mut stub, 0, &[0xFF, 0xFF]);

        // ACT
        let result = extract_metadata(&stub);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn extract_metadata_missing_dos_header() {
        // ARRANGE
        let stub = [];

        // ACT
        let result = extract_metadata(&stub);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn validate_section_header_capacity_accepts_available_space() {
        // ARRANGE
        let metadata = PeMetadata {
            file_header_offset: 64,
            optional_header_offset: 88,
            section_table_offset: 328,
            size_of_headers: 512,
            section_alignment: 4096,
            file_alignment: 512,
            last_section_file_end: 512,
            last_section_virtual_end: 4096,
            current_section_count: 1,
        };

        // ACT
        let result = validate_section_header_capacity(&metadata, 3);

        // ASSERT
        assert!(
            result.is_ok(),
            "section header capacity should accept available space"
        );
    }
    #[test]
    fn validate_section_header_capacity_rejects_expansion_past_headers() {
        // ARRANGE
        let metadata = PeMetadata {
            file_header_offset: 64,
            optional_header_offset: 88,
            section_table_offset: 328,
            size_of_headers: 368,
            section_alignment: 4096,
            file_alignment: 512,
            last_section_file_end: 512,
            last_section_virtual_end: 4096,
            current_section_count: 1,
        };

        // ACT
        let result = validate_section_header_capacity(&metadata, 2);

        // ASSERT
        assert!(
            matches!(result, Err(YukiError::InvalidPeStructure(message)) if message.contains("section table exceeds size of headers"))
        );
    }
}
