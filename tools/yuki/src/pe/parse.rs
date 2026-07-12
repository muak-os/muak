//! PE file parsing and manipulation.

use std::io::Read;

use object::pe::ImageSectionHeader;

use crate::align;
use crate::error::{Result, YukiError};

const COFF_NUMBER_OF_SECTIONS_OFFSET: usize = 2;
const OPT_HEADER_SIZE_OF_IMAGE_OFFSET: usize = 56;

/// Metadata extracted from a PE file header.
#[derive(Debug, Clone, PartialEq)]
pub struct Metadata {
    /// Offset to the COFF file header.
    pub file_header_offset: usize,
    /// Offset to the optional header.
    pub optional_header_offset: usize,
    /// Offset to the section table.
    pub section_table_offset: usize,
    /// Size of all headers combined.
    pub size_of_headers: u32,
    /// Alignment of sections in memory.
    pub section_alignment: u32,
    /// Alignment of sections in the file.
    pub file_alignment: u32,
    /// File offset of the end of the last section.
    pub last_section_file_end: u32,
    /// Virtual address of the end of the last section.
    pub last_section_virtual_end: u32,
    /// Number of sections in the section table.
    pub existing_section_count: u16,
}

/// Extracts metadata from a PE file reader.
///
/// # Errors
///
/// Returns an error if the PE file is malformed or cannot be parsed.
pub fn extract_metadata(reader: &mut dyn Read) -> Result<(Metadata, Vec<u8>)> {
    let mut buf = read_prefix(reader)?;
    let size_of_headers = parse_size_of_headers(&buf)?;
    if size_of_headers > buf.len() {
        buf.resize(size_of_headers, 0);
        let Some(tail) = buf.get_mut(512..size_of_headers) else {
            return Err(YukiError::PeParseError(
                "buffer range overflow for PE headers".to_owned(),
            ));
        };
        if let Err(e) = reader.read_exact(tail) {
            return Err(YukiError::PeParseError(format!(
                "Failed to read full PE headers: {e}"
            )));
        }
    }
    if size_of_headers > 0 && size_of_headers < buf.len() {
        buf.truncate(size_of_headers);
    }
    let metadata = parse_metadata(&buf)?;

    Ok((metadata, buf))
}

fn parse_metadata(buf: &[u8]) -> Result<Metadata> {
    let pe_offset = parse_e_lfanew(buf)?;

    let Some(coff_header_offset) = pe_offset.checked_add(4) else {
        return Err(YukiError::PeParseError(
            "coff header offset overflow".to_owned(),
        ));
    };
    let Some(optional_header_offset) = coff_header_offset.checked_add(20) else {
        return Err(YukiError::PeParseError(
            "optional header offset overflow".to_owned(),
        ));
    };

    let min_opt_hdr_size = 64_usize;
    if buf.len() < optional_header_offset.saturating_add(min_opt_hdr_size) {
        return Err(YukiError::PeParseError(
            "PE file too small to contain required headers".to_owned(),
        ));
    }

    if buf.first() != Some(&b'M') || buf.get(1) != Some(&b'Z') {
        return Err(YukiError::PeParseError("Invalid DOS signature".to_owned()));
    }

    let Some(pe_sig_end) = pe_offset.checked_add(4) else {
        return Err(YukiError::PeParseError(
            "pe signature range overflow".to_owned(),
        ));
    };
    if buf.get(pe_offset..pe_sig_end) != Some(b"PE\0\0") {
        return Err(YukiError::PeParseError("Invalid PE signature".to_owned()));
    }

    let num_sections = u16_from_le(
        buf,
        coff_header_offset.saturating_add(COFF_NUMBER_OF_SECTIONS_OFFSET),
    )?;
    let size_of_opt_hdr = usize::from(u16_from_le(buf, coff_header_offset.saturating_add(16))?);

    let section_alignment = u32_from_le(buf, optional_header_offset.saturating_add(32))?;
    let file_alignment = u32_from_le(buf, optional_header_offset.saturating_add(36))?;
    let size_of_headers = u32_from_le(buf, optional_header_offset.saturating_add(60))?;

    validate_pe_params(section_alignment, file_alignment, size_of_headers)?;

    let section_table_offset = optional_header_offset.saturating_add(size_of_opt_hdr);
    if section_table_offset.saturating_add(usize::from(num_sections).saturating_mul(40)) > buf.len()
    {
        return Err(YukiError::InvalidPeStructure(
            "section table exceeds size of headers".to_owned(),
        ));
    }

    let (last_section_file_end, last_section_virtual_end) =
        find_section_ends(buf, section_table_offset, num_sections, section_alignment)?;

    Ok(Metadata {
        file_header_offset: coff_header_offset,
        optional_header_offset,
        section_table_offset,
        size_of_headers,
        section_alignment,
        file_alignment,
        last_section_file_end,
        last_section_virtual_end,
        existing_section_count: num_sections,
    })
}

pub(crate) fn section_count_offset(metadata: &Metadata) -> usize {
    metadata
        .file_header_offset
        .saturating_add(COFF_NUMBER_OF_SECTIONS_OFFSET)
}

pub(crate) fn size_of_image_offset(metadata: &Metadata) -> usize {
    metadata
        .optional_header_offset
        .saturating_add(OPT_HEADER_SIZE_OF_IMAGE_OFFSET)
}

/// Validates that the section header table has capacity for additional sections.
///
/// # Errors
///
/// Returns an error if adding the additional sections would exceed the header capacity.
pub fn validate_section_header_capacity(
    metadata: &Metadata,
    additional_sections: usize,
) -> Result<()> {
    let total_sections =
        usize::from(metadata.existing_section_count).saturating_add(additional_sections);
    let section_table_size =
        total_sections.saturating_mul(core::mem::size_of::<ImageSectionHeader>());
    let section_table_end = metadata
        .section_table_offset
        .saturating_add(section_table_size);
    let Ok(size_of_headers) = usize::try_from(metadata.size_of_headers) else {
        return Err(YukiError::InvalidPeStructure(
            "size of headers does not fit in usize".to_owned(),
        ));
    };

    if section_table_end > size_of_headers {
        return Err(YukiError::InvalidPeStructure(format!(
            "section table exceeds size of headers: {section_table_end}>{size_of_headers}"
        )));
    }

    Ok(())
}

fn parse_size_of_headers(buf: &[u8]) -> Result<usize> {
    let pe_offset = parse_e_lfanew(buf)?;

    let soh_offset = pe_offset.saturating_add(24).saturating_add(60);
    let Some(soh_bytes) = buf.get(soh_offset..soh_offset.saturating_add(4)) else {
        return Err(YukiError::PeParseError(
            "Optional header too small to contain size_of_headers".to_owned(),
        ));
    };

    let mut soh_arr = [0_u8; 4];
    soh_arr.copy_from_slice(soh_bytes);

    usize::try_from(u32::from_le_bytes(soh_arr))
        .map_err(|_err| YukiError::PeParseError("size_of_headers overflow".to_owned()))
}

fn parse_e_lfanew(buf: &[u8]) -> Result<usize> {
    let e_lfanew = u32_from_le(buf, 0x3C)?;

    usize::try_from(e_lfanew)
        .map_err(|_err| YukiError::PeParseError("e_lfanew overflow".to_owned()))
}

fn read_prefix(reader: &mut dyn Read) -> Result<Vec<u8>> {
    let mut buf = vec![0_u8; 512];
    if let Err(e) = reader.read_exact(&mut buf) {
        return Err(YukiError::PeParseError(format!(
            "Failed to read PE headers: {e}"
        )));
    }

    Ok(buf)
}

fn find_section_ends(
    buf: &[u8],
    section_table_offset: usize,
    num_sections: u16,
    section_alignment: u32,
) -> Result<(u32, u32)> {
    (0..num_sections).try_fold((0_u32, 0_u32), |(max_file, max_virt), i| {
        let section_offset = section_table_offset.saturating_add(usize::from(i).saturating_mul(40));
        let ptr_raw_data = u32_from_le(buf, section_offset.saturating_add(20))?;
        let size_raw_data = u32_from_le(buf, section_offset.saturating_add(16))?;
        let virtual_size = u32_from_le(buf, section_offset.saturating_add(8))?;
        let virtual_addr = u32_from_le(buf, section_offset.saturating_add(12))?;

        let Some(file_end) = ptr_raw_data.checked_add(size_raw_data) else {
            return Err(YukiError::InvalidPeStructure(
                "section raw data end overflow".to_owned(),
            ));
        };
        let aligned_virtual_size = align::to(virtual_size, section_alignment);
        let Some(virt_end) = virtual_addr.checked_add(aligned_virtual_size) else {
            return Err(YukiError::InvalidPeStructure(
                "section virtual end overflow".to_owned(),
            ));
        };

        Ok::<(u32, u32), YukiError>((max_file.max(file_end), max_virt.max(virt_end)))
    })
}

fn validate_pe_params(
    section_alignment: u32,
    file_alignment: u32,
    size_of_headers: u32,
) -> Result<()> {
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

    Ok(())
}

fn u32_from_le(buf: &[u8], offset: usize) -> Result<u32> {
    let Some(end) = offset.checked_add(4) else {
        return Err(YukiError::PeParseError("u32 offset overflow".to_owned()));
    };
    let Some(bytes) = buf.get(offset..end) else {
        return Err(YukiError::PeParseError(
            "buffer too small for u32".to_owned(),
        ));
    };
    let mut arr = [0_u8; 4];
    arr.copy_from_slice(bytes);

    Ok(u32::from_le_bytes(arr))
}

fn u16_from_le(buf: &[u8], offset: usize) -> Result<u16> {
    let Some(end) = offset.checked_add(2) else {
        return Err(YukiError::PeParseError("u16 offset overflow".to_owned()));
    };
    let Some(bytes) = buf.get(offset..end) else {
        return Err(YukiError::PeParseError(
            "buffer too small for u16".to_owned(),
        ));
    };
    let mut arr = [0_u8; 2];
    arr.copy_from_slice(bytes);

    Ok(u16::from_le_bytes(arr))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn write_bytes(bytes: &mut [u8], offset: usize, data: &[u8]) {
        let end = offset.saturating_add(data.len());
        if let Some(dst) = bytes.get_mut(offset..end) {
            dst.copy_from_slice(data);
        }
    }

    fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
        write_bytes(bytes, offset, &value.to_le_bytes());
    }

    fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
        write_bytes(bytes, offset, &value.to_le_bytes());
    }

    #[test]
    fn metadata_structure() {
        // ARRANGE
        let metadata = Metadata {
            file_header_offset: 64,
            optional_header_offset: 84,
            section_table_offset: 324,
            size_of_headers: 512,
            section_alignment: 4096,
            file_alignment: 512,
            last_section_file_end: 512,
            last_section_virtual_end: 4096,
            existing_section_count: 1,
        };

        // ACT & ASSERT
        assert_eq!(metadata.file_header_offset, 64);
        assert_eq!(metadata.optional_header_offset, 84);
        assert_eq!(metadata.section_alignment, 4096);
        assert_eq!(metadata.existing_section_count, 1);
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
        let result = extract_metadata(&mut Cursor::new(stub));
        result.unwrap_err();
    }

    #[test]
    fn extract_metadata_too_small() {
        // ARRANGE
        let stub = [0x4D, 0x5A];
        let result = extract_metadata(&mut Cursor::new(stub));
        result.unwrap_err();
    }

    #[test]
    fn extract_metadata_with_malformed_header() {
        let mut stub = vec![0_u8; 100];
        write_bytes(&mut stub, 0, &[0xFF, 0xFF]);
        let result = extract_metadata(&mut Cursor::new(stub));
        result.unwrap_err();
    }

    #[test]
    fn extract_metadata_missing_dos_header() {
        let stub = [];
        let result = extract_metadata(&mut Cursor::new(stub));
        result.unwrap_err();
    }

    #[test]
    fn validate_section_header_capacity_accepts_available_space() {
        let metadata = Metadata {
            file_header_offset: 64,
            optional_header_offset: 88,
            section_table_offset: 328,
            size_of_headers: 512,
            section_alignment: 4096,
            file_alignment: 512,
            last_section_file_end: 512,
            last_section_virtual_end: 4096,
            existing_section_count: 1,
        };

        let result = validate_section_header_capacity(&metadata, 3);
        assert!(
            result.is_ok(),
            "section header capacity should accept available space"
        );
    }

    #[test]
    fn validate_section_header_capacity_rejects_expansion_past_headers() {
        let metadata = Metadata {
            file_header_offset: 64,
            optional_header_offset: 88,
            section_table_offset: 328,
            size_of_headers: 368,
            section_alignment: 4096,
            file_alignment: 512,
            last_section_file_end: 512,
            last_section_virtual_end: 4096,
            existing_section_count: 1,
        };

        // ACT
        let result = validate_section_header_capacity(&metadata, 2);

        // ASSERT
        assert!(
            matches!(result, Err(YukiError::InvalidPeStructure(message)) if message.contains("section table exceeds size of headers"))
        );
    }

    #[test]
    fn extract_metadata_parses_valid_stub() {
        let mut stub = vec![0_u8; 512];
        write_bytes(&mut stub, 0, b"MZ");
        write_u32(&mut stub, 0x3C, 64);
        write_bytes(&mut stub, 64, b"PE\0\0");
        write_u16(&mut stub, 68, 0x8664);
        write_u16(&mut stub, 70, 1);
        write_u16(&mut stub, 84, 240);
        write_u16(&mut stub, 86, 0x0222);
        let opt_start = 88;
        write_u16(&mut stub, opt_start, 0x020B);
        write_u32(&mut stub, opt_start + 32, 4096);
        write_u32(&mut stub, opt_start + 36, 512);
        write_u32(&mut stub, opt_start + 56, 8192);
        write_u32(&mut stub, opt_start + 60, 512);
        write_u16(&mut stub, opt_start + 68, 10);
        let section_start = opt_start + 240;
        write_bytes(&mut stub, section_start, b".text");
        write_u32(&mut stub, section_start + 8, 16);
        write_u32(&mut stub, section_start + 12, 4096);
        write_u32(&mut stub, section_start + 16, 512);
        write_u32(&mut stub, section_start + 20, 512);
        write_u32(&mut stub, section_start + 36, 0x6000_0020);

        let (metadata, prefix) =
            extract_metadata(&mut Cursor::new(stub)).expect("valid stub should parse");
        assert_eq!(metadata.file_header_offset, 68);
        assert_eq!(metadata.optional_header_offset, 88);
        assert_eq!(metadata.section_table_offset, 328);
        assert_eq!(metadata.size_of_headers, 512);
        assert_eq!(metadata.section_alignment, 4096);
        assert_eq!(metadata.file_alignment, 512);
        assert_eq!(metadata.existing_section_count, 1);
        assert!(!prefix.is_empty());
    }

    #[test]
    fn parse_metadata_rejects_small_buffer() {
        let mut buf = vec![0_u8; 100];
        write_bytes(&mut buf, 0, b"MZ");
        write_u32(&mut buf, 0x3C, 64);
        let result = parse_metadata(&buf);
        assert!(matches!(result, Err(YukiError::PeParseError(msg)) if msg.contains("too small")));
    }

    #[test]
    fn parse_metadata_rejects_invalid_dos_signature() {
        let mut buf = vec![0_u8; 200];
        write_bytes(&mut buf, 0, b"XX");
        write_u32(&mut buf, 0x3C, 64);
        write_bytes(&mut buf, 64, b"PE\0\0");
        let result = parse_metadata(&buf);
        assert!(
            matches!(result, Err(YukiError::PeParseError(msg)) if msg.contains("DOS signature"))
        );
    }

    #[test]
    fn parse_metadata_rejects_invalid_pe_signature() {
        let mut buf = vec![0_u8; 200];
        write_bytes(&mut buf, 0, b"MZ");
        write_u32(&mut buf, 0x3C, 64);
        write_bytes(&mut buf, 64, b"XX\0\0");
        let result = parse_metadata(&buf);
        assert!(
            matches!(result, Err(YukiError::PeParseError(msg)) if msg.contains("PE signature"))
        );
    }

    #[test]
    fn parse_metadata_rejects_section_table_overflow() {
        let mut buf = vec![0_u8; 200];
        write_bytes(&mut buf, 0, b"MZ");
        write_u32(&mut buf, 0x3C, 64);
        write_bytes(&mut buf, 64, b"PE\0\0");
        write_u16(&mut buf, 70, 100);
        write_u16(&mut buf, 84, 240);
        write_u16(&mut buf, 86, 0x0002);
        write_u16(&mut buf, 88, 0x020B);
        write_u32(&mut buf, 88 + 32, 4096);
        write_u32(&mut buf, 88 + 36, 512);
        write_u32(&mut buf, 88 + 60, 512);
        let result = parse_metadata(&buf);
        assert!(
            matches!(result, Err(YukiError::InvalidPeStructure(msg)) if msg.contains("section table exceeds size of headers"))
        );
    }

    #[test]
    fn extract_metadata_rejects_truncated_stub_with_large_headers() {
        // ARRANGE
        let mut stub = vec![0_u8; 1024];
        write_bytes(&mut stub, 0, b"MZ");
        write_u32(&mut stub, 0x3C, 64);
        write_bytes(&mut stub, 64, b"PE\0\0");
        write_u16(&mut stub, 68, 0x8664);
        write_u16(&mut stub, 70, 1);
        write_u16(&mut stub, 84, 240);
        write_u16(&mut stub, 86, 0x0222);
        write_u16(&mut stub, 88, 0x020B);
        write_u32(&mut stub, 88 + 32, 4096);
        write_u32(&mut stub, 88 + 36, 512);
        write_u32(&mut stub, 88 + 60, 1024);
        // Truncate to 600 so read_exact into the extended prefix fails
        stub.truncate(600);

        // ACT
        let result = extract_metadata(&mut Cursor::new(stub));

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn extract_metadata_rejects_large_e_lfanew() {
        // ARRANGE
        let mut stub = vec![0_u8; 512];
        write_bytes(&mut stub, 0, b"MZ");
        // e_lfanew = 500 causes soh_offset = 500 + 84 = 584 > 512 - 4
        write_u32(&mut stub, 0x3C, 500);

        // ACT
        let result = extract_metadata(&mut Cursor::new(stub));

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::PeParseError(msg))
                if msg.contains("Optional header too small")
        ));
    }

    #[test]
    fn u32_from_le_rejects_small_buffer() {
        // ARRANGE
        let buf = [0_u8; 3];

        // ACT
        let result = u32_from_le(&buf, 0);

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::PeParseError(msg))
                if msg.contains("buffer too small for u32")
        ));
    }

    #[test]
    fn u16_from_le_rejects_small_buffer() {
        // ARRANGE
        let buf = [0_u8; 1];

        // ACT
        let result = u16_from_le(&buf, 0);

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::PeParseError(msg))
                if msg.contains("buffer too small for u16")
        ));
    }

    #[test]
    fn parse_metadata_rejects_missing_coff_header() {
        // ARRANGE
        let mut buf = vec![0_u8; 128];
        write_bytes(&mut buf, 0, b"MZ");
        write_u32(&mut buf, 0x3C, 64);

        // ACT
        let result = parse_metadata(&buf);

        // ASSERT
        result.unwrap_err();
    }
}
