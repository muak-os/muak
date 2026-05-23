//! PE file parsing and manipulation.

use std::mem;

use object::LittleEndian as LE;
use object::pe::{ImageFileHeader, ImageSectionHeader};
use object::read::pe::{ImageNtHeaders, PeFile64};

use crate::YukiError;
use crate::binary;
use crate::binary::{align_to, read_u32, usize_from_u128};
use crate::constants;

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

/// Extracts PE metadata from the given stub data, validating the structure and returning relevant offsets and alignment information
pub fn extract_metadata(stub_data: &[u8]) -> Result<PeMetadata, YukiError> {
    let pe = PeFile64::parse(stub_data)
        .map_err(|err| YukiError::PeParseError(format!("Invalid PE file format: {err}")))?;
    let nt_headers = pe.nt_headers();
    let sections = pe.section_table();

    let pe_offset = usize_from_u128(
        u128::from(read_u32(stub_data, constants::DOS_HEADER_PE_OFFSET)?),
        "PE offset does not fit in usize",
    )?;
    let file_header_offset = pe_offset.saturating_add(constants::PE_SIGNATURE_SIZE);
    let optional_header_offset =
        file_header_offset.saturating_add(mem::size_of::<ImageFileHeader>());
    let optional_header_size =
        usize::from(nt_headers.file_header().size_of_optional_header.get(LE));
    let section_table_offset = optional_header_offset.saturating_add(optional_header_size);

    let section_alignment_offset =
        optional_header_offset.saturating_add(constants::OPT_HEADER_SECTION_ALIGNMENT);
    let file_alignment_offset =
        optional_header_offset.saturating_add(constants::OPT_HEADER_FILE_ALIGNMENT);
    let size_of_headers_offset = optional_header_offset.saturating_add(60);

    let section_alignment = read_u32(stub_data, section_alignment_offset)?;
    let file_alignment = read_u32(stub_data, file_alignment_offset)?;
    let size_of_headers = read_u32(stub_data, size_of_headers_offset)?;

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
            "invalid size of headers 0".to_string(),
        ));
    }

    let (last_section_file_end, last_section_virtual_end) =
        sections
            .iter()
            .try_fold((0_u32, 0_u32), |(max_file, max_virt), s| {
                let file_end = s
                    .pointer_to_raw_data
                    .get(LE)
                    .checked_add(s.size_of_raw_data.get(LE))
                    .ok_or_else(|| {
                        YukiError::InvalidPeStructure("section raw data end overflow".to_string())
                    })?;
                let aligned_virtual_size = align_to(s.virtual_size.get(LE), section_alignment);
                let virt_end = s
                    .virtual_address
                    .get(LE)
                    .checked_add(aligned_virtual_size)
                    .ok_or_else(|| {
                        YukiError::InvalidPeStructure("section virtual end overflow".to_string())
                    })?;
                Ok((max_file.max(file_end), max_virt.max(virt_end)))
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

/// Updates the `SizeOfImage` field in the PE optional header to accommodate the specified maximum virtual end address
pub fn update_image_size(
    stub_data: &mut [u8],
    metadata: &PeMetadata,
    max_virtual_end: u32,
) -> Result<(), YukiError> {
    let size_of_image_off = metadata
        .optional_header_offset
        .saturating_add(constants::OPT_HEADER_SIZE_OF_IMAGE);
    let new_size_of_image = align_to(max_virtual_end, metadata.section_alignment);
    binary::write_u32(stub_data, size_of_image_off, new_size_of_image)
}

/// Validates that the section header table can accommodate the specified number of additional sections
pub fn validate_section_header_capacity(
    metadata: &PeMetadata,
    additional_sections: usize,
) -> Result<(), YukiError> {
    let total_sections =
        usize::from(metadata.current_section_count).saturating_add(additional_sections);
    let section_table_size = total_sections.saturating_mul(mem::size_of::<ImageSectionHeader>());
    let section_table_end = metadata
        .section_table_offset
        .saturating_add(section_table_size);
    let size_of_headers = usize_from_u128(
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
    use object::pe;

    use super::*;
    use crate::binary;
    use crate::constants;

    fn generate_minimal_stub() -> Vec<u8> {
        const DOS_HEADER_SIZE: usize = 64;
        const PE_SIGNATURE_SIZE: usize = 4;
        const COFF_HEADER_SIZE: usize = 20;
        const OPTIONAL_HEADER_SIZE: usize = 240;
        const SECTION_HEADER_SIZE: usize = 40;
        const FILE_ALIGNMENT: usize = 512;
        const SECTION_ALIGNMENT: usize = 4096;
        const EXTRA_SECTION_HEADER_SLOTS: usize = 4;

        fn align_up(value: usize, alignment: usize) -> usize {
            (value + alignment - 1) & !(alignment - 1)
        }

        fn write_pe_headers(buf: &mut [u8], coff_offset: usize) {
            let mut off = coff_offset;
            buf[off..off + 4].copy_from_slice(b"PE\0\0");
            off += PE_SIGNATURE_SIZE;

            buf[off..off + 2].copy_from_slice(&pe::IMAGE_FILE_MACHINE_AMD64.to_le_bytes());
            off += 2;
            buf[off..off + 2].copy_from_slice(&1u16.to_le_bytes());
            off += 2;
            off += 12;
            buf[off..off + 2].copy_from_slice(&(OPTIONAL_HEADER_SIZE as u16).to_le_bytes());
            off += 2;
            let characteristics: u16 = pe::IMAGE_FILE_EXECUTABLE_IMAGE
                | pe::IMAGE_FILE_LARGE_ADDRESS_AWARE
                | pe::IMAGE_FILE_DLL;
            buf[off..off + 2].copy_from_slice(&characteristics.to_le_bytes());
            off += 2;

            let opt_off = off;
            buf[off..off + 2].copy_from_slice(&pe::IMAGE_NT_OPTIONAL_HDR64_MAGIC.to_le_bytes());
            off += 2;
            off += 2;
            let section_size = FILE_ALIGNMENT;
            buf[off..off + 4].copy_from_slice(&(section_size as u32).to_le_bytes());
            off += 4;
            off += 8;
            buf[off..off + 4].copy_from_slice(&(SECTION_ALIGNMENT as u32).to_le_bytes());
            off += 4;
            buf[off..off + 4].copy_from_slice(&(SECTION_ALIGNMENT as u32).to_le_bytes());
            off += 4;
            buf[off..off + 8].copy_from_slice(&0x10000u64.to_le_bytes());
            off += 8;
            buf[off..off + 4].copy_from_slice(&(SECTION_ALIGNMENT as u32).to_le_bytes());
            off += 4;
            buf[off..off + 4].copy_from_slice(&(FILE_ALIGNMENT as u32).to_le_bytes());
            off += 4;
            off += 16;
            let size_of_image = (SECTION_ALIGNMENT * 2) as u32;
            buf[off..off + 4].copy_from_slice(&size_of_image.to_le_bytes());
            off += 4;
            let headers_aligned = align_up(
                opt_off
                    + OPTIONAL_HEADER_SIZE
                    + (1 + EXTRA_SECTION_HEADER_SLOTS) * SECTION_HEADER_SIZE,
                FILE_ALIGNMENT,
            ) as u32;
            buf[off..off + 4].copy_from_slice(&headers_aligned.to_le_bytes());
            off += 4;
            off += 4;
            buf[off..off + 2].copy_from_slice(&pe::IMAGE_SUBSYSTEM_EFI_APPLICATION.to_le_bytes());
            off += 2;
            off += 2 + 8 + 8 + 8 + 8 + 4;
            buf[off..off + 4].copy_from_slice(&0u32.to_le_bytes());

            let sh_base = opt_off + OPTIONAL_HEADER_SIZE;
            buf[sh_base..sh_base + 5].copy_from_slice(b".text");
            buf[sh_base + 8..sh_base + 12].copy_from_slice(&(section_size as u32).to_le_bytes());
            buf[sh_base + 12..sh_base + 16]
                .copy_from_slice(&(SECTION_ALIGNMENT as u32).to_le_bytes());
            buf[sh_base + 16..sh_base + 20].copy_from_slice(&(section_size as u32).to_le_bytes());
            let section_rva = align_up(
                opt_off + OPTIONAL_HEADER_SIZE + SECTION_HEADER_SIZE,
                FILE_ALIGNMENT,
            ) as u32;
            buf[sh_base + 20..sh_base + 24].copy_from_slice(&section_rva.to_le_bytes());
            buf[sh_base + 36..sh_base + 40].copy_from_slice(&0x60000020u32.to_le_bytes());
        }

        let headers_raw = DOS_HEADER_SIZE
            + PE_SIGNATURE_SIZE
            + COFF_HEADER_SIZE
            + OPTIONAL_HEADER_SIZE
            + (1 + EXTRA_SECTION_HEADER_SLOTS) * SECTION_HEADER_SIZE;
        let headers_aligned = align_up(headers_raw, FILE_ALIGNMENT);
        let total_size = headers_aligned + FILE_ALIGNMENT;

        let mut stub = vec![0u8; total_size];
        stub[0] = b'M';
        stub[1] = b'Z';
        stub[0x3C..0x40].copy_from_slice(&(DOS_HEADER_SIZE as u32).to_le_bytes());
        write_pe_headers(&mut stub, DOS_HEADER_SIZE);
        stub[headers_aligned] = 0xC3;
        stub
    }

    #[test]
    fn binary_helpers_are_callable() {
        // ARRANGE
        let mut buf = [0u8; 4];

        // ACT
        binary::write_u32(&mut buf, 0, 0x12345678).unwrap_or_default();

        // ASSERT
        assert_eq!(binary::read_u32(&buf, 0).unwrap_or_default(), 0x12345678);
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
        let stub = vec![0u8; 100];

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
        let mut stub = vec![0u8; 100];
        stub[0] = 0xFF;
        stub[1] = 0xFF;

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
    fn extract_metadata_valid_stub() {
        // ARRANGE
        let stub = generate_minimal_stub();

        // ACT
        let metadata = extract_metadata(&stub)
            .unwrap_or_else(|err| panic!("expected valid PE metadata, got {err:?}"));

        // ASSERT
        assert_eq!(metadata.file_header_offset, 68);
        assert_eq!(metadata.optional_header_offset, 88);
        assert_eq!(metadata.section_table_offset, 328);
        assert_eq!(metadata.size_of_headers, 1024);
        assert_eq!(metadata.section_alignment, 4096);
        assert_eq!(metadata.file_alignment, 512);
        assert_eq!(metadata.last_section_file_end, 1024);
        assert_eq!(metadata.last_section_virtual_end, 8192);
        assert_eq!(metadata.current_section_count, 1);
    }

    #[test]
    fn update_pe_image_size_basic() {
        // ARRANGE
        let mut stub = vec![0u8; 1000];
        let metadata = PeMetadata {
            file_header_offset: 0,
            optional_header_offset: 0,
            section_table_offset: 0,
            size_of_headers: 512,
            section_alignment: 4096,
            file_alignment: 512,
            last_section_file_end: 512,
            last_section_virtual_end: 4096,
            current_section_count: 1,
        };

        // ACT
        update_image_size(&mut stub, &metadata, 16384).unwrap_or_default();
        let off = metadata.optional_header_offset + constants::OPT_HEADER_SIZE_OF_IMAGE;
        let written = u32::from_le_bytes([stub[off], stub[off + 1], stub[off + 2], stub[off + 3]]);

        // ASSERT
        assert_eq!(written, 16384);
    }

    #[test]
    fn update_image_size_aligns_to_section_alignment() {
        // ARRANGE
        let mut stub = vec![0u8; 1000];
        let metadata = PeMetadata {
            file_header_offset: 0,
            optional_header_offset: 0,
            section_table_offset: 0,
            size_of_headers: 512,
            section_alignment: 4096,
            file_alignment: 512,
            last_section_file_end: 512,
            last_section_virtual_end: 4096,
            current_section_count: 1,
        };
        let size = 5000;

        // ACT
        update_image_size(&mut stub, &metadata, size).unwrap_or_default();
        let off = metadata.optional_header_offset + constants::OPT_HEADER_SIZE_OF_IMAGE;
        let written = u32::from_le_bytes([stub[off], stub[off + 1], stub[off + 2], stub[off + 3]]);

        // ASSERT
        assert_eq!(written, binary::align_to(size, metadata.section_alignment));
    }

    #[test]
    fn update_image_size_out_of_bounds() {
        // ARRANGE
        let mut stub = vec![0u8; 100];
        let metadata = PeMetadata {
            file_header_offset: 0,
            optional_header_offset: 0,
            section_table_offset: 0,
            size_of_headers: 512,
            section_alignment: 4096,
            file_alignment: 512,
            last_section_file_end: 0,
            last_section_virtual_end: 0,
            current_section_count: 0,
        };

        // ACT
        let result = update_image_size(&mut stub, &metadata, 10000);

        // ASSERT
        assert!(result.is_ok());
        assert_eq!(stub.len(), 100);
    }

    #[test]
    fn update_image_size_rejects_out_of_bounds_write() {
        // ARRANGE
        let mut stub = vec![0u8; 59];
        let metadata = PeMetadata {
            file_header_offset: 0,
            optional_header_offset: 0,
            section_table_offset: 0,
            size_of_headers: 512,
            section_alignment: 4096,
            file_alignment: 512,
            last_section_file_end: 0,
            last_section_virtual_end: 0,
            current_section_count: 0,
        };

        // ACT
        let result = update_image_size(&mut stub, &metadata, 4096);

        // ASSERT
        assert!(
            matches!(result, Err(YukiError::InvalidPeStructure(message)) if message.contains("u32 write oob"))
        );
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
        assert!(result.is_ok());
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

    #[test]
    fn extract_metadata_rejects_invalid_alignment() {
        // ARRANGE
        let mut stub = generate_minimal_stub();
        let file_alignment_offset = 88 + constants::OPT_HEADER_FILE_ALIGNMENT;
        stub[file_alignment_offset..file_alignment_offset + 4].copy_from_slice(&3u32.to_le_bytes());

        // ACT
        let result = extract_metadata(&stub);

        // ASSERT
        assert!(
            matches!(result, Err(YukiError::InvalidPeStructure(message)) if message.contains("invalid file alignment"))
        );
    }

    #[test]
    fn extract_metadata_rejects_zero_section_alignment() {
        // ARRANGE
        let mut stub = generate_minimal_stub();
        let section_alignment_offset = 88 + constants::OPT_HEADER_SECTION_ALIGNMENT;
        stub[section_alignment_offset..section_alignment_offset + 4]
            .copy_from_slice(&0u32.to_le_bytes());

        // ACT
        let result = extract_metadata(&stub);

        // ASSERT
        assert!(
            matches!(result, Err(YukiError::InvalidPeStructure(message)) if message.contains("invalid section alignment"))
        );
    }

    #[test]
    fn extract_metadata_rejects_zero_size_of_headers() {
        // ARRANGE
        let mut stub = generate_minimal_stub();
        stub[148..152].copy_from_slice(&0u32.to_le_bytes());

        // ACT
        let result = extract_metadata(&stub);

        // ASSERT
        assert!(
            matches!(result, Err(YukiError::InvalidPeStructure(message)) if message.contains("invalid size of headers 0"))
        );
    }

    #[test]
    fn extract_metadata_rejects_section_raw_data_overflow() {
        // ARRANGE
        let mut stub = generate_minimal_stub();
        let section_offset = 328;
        stub[section_offset + 16..section_offset + 20].copy_from_slice(&u32::MAX.to_le_bytes());
        stub[section_offset + 20..section_offset + 24].copy_from_slice(&1u32.to_le_bytes());

        // ACT
        let result = extract_metadata(&stub);

        // ASSERT
        assert!(
            matches!(result, Err(YukiError::InvalidPeStructure(message)) if message.contains("section raw data end overflow"))
        );
    }

    #[test]
    fn extract_metadata_rejects_section_virtual_end_overflow() {
        // ARRANGE
        let mut stub = generate_minimal_stub();
        let section_offset = 328;
        stub[section_offset + 8..section_offset + 12].copy_from_slice(&1u32.to_le_bytes());
        stub[section_offset + 12..section_offset + 16].copy_from_slice(&u32::MAX.to_le_bytes());

        // ACT
        let result = extract_metadata(&stub);

        // ASSERT
        assert!(
            matches!(result, Err(YukiError::InvalidPeStructure(message)) if message.contains("section virtual end overflow"))
        );
    }
}
