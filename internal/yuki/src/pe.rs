use object::LittleEndian as LE;
use object::pe::ImageFileHeader;
use object::read::pe::{ImageNtHeaders, PeFile64};
use std::mem;

use crate::YukiError;
use crate::binary;
use crate::binary::{align_to, read_u32};
use crate::config;

pub struct PeMetadata {
    pub file_header_offset: usize,
    pub optional_header_offset: usize,
    pub section_table_offset: usize,
    pub section_alignment: u32,
    pub file_alignment: u32,
    pub last_section_file_end: u32,
    pub last_section_virtual_end: u32,
    pub current_section_count: u16,
}

pub fn extract_metadata(stub_data: &[u8]) -> Result<PeMetadata, YukiError> {
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

pub fn update_image_size(stub_data: &mut [u8], metadata: &PeMetadata, max_virtual_end: u32) {
    let size_of_image_off = metadata.optional_header_offset + config::OPT_HEADER_SIZE_OF_IMAGE;
    let new_size_of_image = align_to(max_virtual_end, metadata.section_alignment);
    binary::write_u32(stub_data, size_of_image_off, new_size_of_image);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pe_metadata_structure() {
        let metadata = PeMetadata {
            file_header_offset: 64,
            optional_header_offset: 84,
            section_table_offset: 324,
            section_alignment: 4096,
            file_alignment: 512,
            last_section_file_end: 512,
            last_section_virtual_end: 4096,
            current_section_count: 1,
        };

        assert_eq!(metadata.file_header_offset, 64);
        assert_eq!(metadata.optional_header_offset, 84);
        assert_eq!(metadata.section_alignment, 4096);
        assert_eq!(metadata.current_section_count, 1);
    }

    #[test]
    fn test_extract_metadata_invalid_pe() {
        let invalid_stub = vec![0u8; 100];
        let result = extract_metadata(&invalid_stub);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_metadata_too_small() {
        let stub = vec![0x4D, 0x5A];
        let result = extract_metadata(&stub);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_metadata_with_malformed_header() {
        let mut stub = vec![0u8; 100];
        stub[0] = 0xFF;
        stub[1] = 0xFF;
        let result = extract_metadata(&stub);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_metadata_missing_dos_header() {
        let stub = vec![];
        let result = extract_metadata(&stub);
        assert!(result.is_err());
    }

    #[test]
    fn test_update_pe_image_size_basic() {
        let mut stub = vec![0u8; 1000];
        let metadata = PeMetadata {
            file_header_offset: 0,
            optional_header_offset: 0,
            section_table_offset: 0,
            section_alignment: 4096,
            file_alignment: 512,
            last_section_file_end: 512,
            last_section_virtual_end: 4096,
            current_section_count: 1,
        };

        let new_size = 16384u32;
        update_image_size(&mut stub, &metadata, new_size);

        let size_of_image_offset =
            metadata.optional_header_offset + config::OPT_HEADER_SIZE_OF_IMAGE;
        let written_size = u32::from_le_bytes([
            stub[size_of_image_offset],
            stub[size_of_image_offset + 1],
            stub[size_of_image_offset + 2],
            stub[size_of_image_offset + 3],
        ]);

        assert_eq!(written_size, 16384);
    }

    #[test]
    fn test_update_image_size_aligns_to_section_alignment() {
        let mut stub = vec![0u8; 1000];
        let metadata = PeMetadata {
            file_header_offset: 0,
            optional_header_offset: 0,
            section_table_offset: 0,
            section_alignment: 4096,
            file_alignment: 512,
            last_section_file_end: 512,
            last_section_virtual_end: 4096,
            current_section_count: 1,
        };

        let unaligned_size = 5000u32;
        update_image_size(&mut stub, &metadata, unaligned_size);

        let size_of_image_offset =
            metadata.optional_header_offset + config::OPT_HEADER_SIZE_OF_IMAGE;
        let written_size = u32::from_le_bytes([
            stub[size_of_image_offset],
            stub[size_of_image_offset + 1],
            stub[size_of_image_offset + 2],
            stub[size_of_image_offset + 3],
        ]);

        let expected = align_to(unaligned_size, metadata.section_alignment);
        assert_eq!(written_size, expected);
    }

    #[test]
    fn test_update_image_size_out_of_bounds() {
        let mut stub = vec![0u8; 100];
        let metadata = PeMetadata {
            file_header_offset: 0,
            optional_header_offset: 0,
            section_table_offset: 0,
            section_alignment: 4096,
            file_alignment: 512,
            last_section_file_end: 0,
            last_section_virtual_end: 0,
            current_section_count: 0,
        };

        update_image_size(&mut stub, &metadata, 10000);
        assert_eq!(stub.len(), 100);
    }

    #[test]
    fn test_update_image_size_zero_alignment() {
        let mut stub = vec![0u8; 1000];
        let metadata = PeMetadata {
            file_header_offset: 0,
            optional_header_offset: 0,
            section_table_offset: 0,
            section_alignment: 0,
            file_alignment: 512,
            last_section_file_end: 512,
            last_section_virtual_end: 4096,
            current_section_count: 1,
        };

        let size = 5000u32;
        update_image_size(&mut stub, &metadata, size);

        let size_of_image_offset =
            metadata.optional_header_offset + config::OPT_HEADER_SIZE_OF_IMAGE;
        let written_size = u32::from_le_bytes([
            stub[size_of_image_offset],
            stub[size_of_image_offset + 1],
            stub[size_of_image_offset + 2],
            stub[size_of_image_offset + 3],
        ]);

        assert_eq!(written_size, size);
    }
}
