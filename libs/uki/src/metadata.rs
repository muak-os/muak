//! Metadata extraction from PE files.

use std::io::Read;

use object::LittleEndian as LE;
use object::pe::ImageFileHeader;
use object::read::pe::PeFile64;

use crate::align;
use crate::error::{Result, UkiError};

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
    /// Number of data directory entries.
    pub num_data_directories: u32,
}

/// Reads `size_of_headers` from raw PE bytes without requiring full parsing.
///
/// # Errors
///
/// Returns `Err` if the buffer is too small to contain `e_lfanew` or the
/// `size_of_headers` field.
pub fn peek_size_of_headers(data: &[u8]) -> Result<u32> {
    let e_lfanew = u32::from_le_bytes(
        data.get(0x3C..0x40)
            .and_then(|slice| <[u8; 4]>::try_from(slice).ok())
            .ok_or(UkiError::InvalidPe("missing e_lfanew"))?,
    );
    let pe_offset = usize::try_from(e_lfanew).map_err(|_source| UkiError::Overflow("PE offset"))?;
    let soh_offset = pe_offset
        .checked_add(24)
        .and_then(|off| off.checked_add(60))
        .ok_or(UkiError::InvalidPe("size_of_headers offset overflow"))?;
    let soh_end = soh_offset
        .checked_add(4)
        .ok_or(UkiError::InvalidPe("size_of_headers range overflow"))?;

    Ok(u32::from_le_bytes(
        data.get(soh_offset..soh_end)
            .and_then(|slice| <[u8; 4]>::try_from(slice).ok())
            .ok_or(UkiError::InvalidPe("size_of_headers out of range"))?,
    ))
}

/// Extracts PE metadata from a `Read` stream and returns the raw header bytes.
///
/// # Errors
///
/// Returns `Err` if the reader returns an I/O error, the buffer is too small to
/// contain the required PE fields, or the PE file is malformed.
pub fn extract(reader: &mut dyn Read) -> Result<(Vec<u8>, Metadata)> {
    let mut buf = vec![0_u8; 512];
    reader.read_exact(&mut buf)?;

    let size_of_headers = usize::try_from(peek_size_of_headers(&buf)?)
        .map_err(|_source| UkiError::Overflow("size_of_headers"))?;

    if size_of_headers > buf.len() {
        buf.resize(size_of_headers, 0);
        let tail = buf
            .get_mut(512..size_of_headers)
            .ok_or(UkiError::InvalidPe("headers buffer bounds"))?;
        reader.read_exact(tail)?;
    }
    if size_of_headers > 0 && size_of_headers < buf.len() {
        buf.truncate(size_of_headers);
    }

    let info = parse(&buf)?;

    Ok((buf, info))
}

/// Parses PE metadata from a complete header buffer.
///
/// # Errors
///
/// Returns `Err` if the buffer is not a valid PE64 file, uses invalid
/// alignments, or declares a zero `size_of_headers`.
pub fn parse(data: &[u8]) -> Result<Metadata> {
    if data.len() < 0x40 {
        return Err(UkiError::InvalidPe("file too small"));
    }

    let pe = PeFile64::parse(data).map_err(|_source| UkiError::InvalidPe("invalid PE format"))?;

    let pe_offset = usize::try_from(pe.dos_header().nt_headers_offset())
        .map_err(|_source| UkiError::Overflow("PE offset"))?;
    let file_header_offset = pe_offset
        .checked_add(4)
        .ok_or(UkiError::Overflow("file header offset"))?;
    let optional_header_offset = file_header_offset
        .checked_add(core::mem::size_of::<ImageFileHeader>())
        .ok_or(UkiError::Overflow("optional header offset"))?;

    let size_of_opt_hdr = usize::from(pe.nt_headers().file_header.size_of_optional_header.get(LE));
    let section_table_offset = optional_header_offset
        .checked_add(size_of_opt_hdr)
        .ok_or(UkiError::Overflow("section table offset"))?;

    let section_alignment = pe.nt_headers().optional_header.section_alignment.get(LE);
    let file_alignment = pe.nt_headers().optional_header.file_alignment.get(LE);
    let size_of_headers = pe.nt_headers().optional_header.size_of_headers.get(LE);
    if section_alignment == 0 || !section_alignment.is_power_of_two() {
        return Err(UkiError::InvalidPe("invalid section alignment"));
    }
    if file_alignment == 0 || !file_alignment.is_power_of_two() {
        return Err(UkiError::InvalidPe("invalid file alignment"));
    }
    if size_of_headers == 0 {
        return Err(UkiError::InvalidPe("invalid size of headers 0"));
    }
    let num_data_directories = pe
        .nt_headers()
        .optional_header
        .number_of_rva_and_sizes
        .get(LE);

    let mut last_section_file_end = size_of_headers;
    let mut last_section_virtual_end = size_of_headers;

    for section in pe.section_table().iter() {
        let ptr = section.pointer_to_raw_data.get(LE);
        let size = section.size_of_raw_data.get(LE);
        if ptr != 0 && size != 0 {
            let file_end = ptr
                .checked_add(size)
                .ok_or(UkiError::Overflow("section file end"))?;
            last_section_file_end = last_section_file_end.max(file_end);
        }

        let vaddr = section.virtual_address.get(LE);
        let vsize = section.virtual_size.get(LE);
        let aligned_vsize = align::to(vsize, section_alignment);
        let vend = vaddr
            .checked_add(aligned_vsize)
            .ok_or(UkiError::Overflow("section virtual end"))?;
        last_section_virtual_end = last_section_virtual_end.max(vend);
    }

    let section_count = pe
        .section_table()
        .iter()
        .count()
        .try_into()
        .map_err(|_source| UkiError::Overflow("section count"))?;

    Ok(Metadata {
        file_header_offset,
        optional_header_offset,
        section_table_offset,
        size_of_headers,
        section_alignment,
        file_alignment,
        last_section_file_end,
        last_section_virtual_end,
        existing_section_count: section_count,
        num_data_directories,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_bytes(buf: &mut [u8], offset: usize, data: &[u8]) {
        let end = offset
            .checked_add(data.len())
            .expect("write_bytes offset overflow");
        buf.get_mut(offset..end)
            .expect("write_bytes range")
            .copy_from_slice(data);
    }

    fn write_u16(buf: &mut [u8], offset: usize, value: u16) {
        write_bytes(buf, offset, &value.to_le_bytes());
    }

    fn write_u32(buf: &mut [u8], offset: usize, value: u32) {
        write_bytes(buf, offset, &value.to_le_bytes());
    }

    fn build_minimal_pe64() -> Vec<u8> {
        let file_alignment = 512_u32;
        let opt_start = 88_usize;
        let section_start = opt_start.checked_add(240).expect("section start");
        let headers_raw = section_start.checked_add(40).expect("headers raw");
        let headers_aligned = u32::try_from(headers_raw)
            .ok()
            .and_then(|value| value.div_ceil(file_alignment).checked_mul(file_alignment))
            .expect("headers aligned");
        let file_alignment_usize = usize::try_from(file_alignment).expect("file align usize");
        let total = usize::try_from(headers_aligned)
            .ok()
            .and_then(|value| value.checked_add(file_alignment_usize))
            .expect("total size");

        let mut buf = vec![0_u8; total];
        write_bytes(&mut buf, 0, b"MZ");
        write_u32(&mut buf, 0x3C, 0x40);
        write_bytes(&mut buf, 0x40, b"PE\0\0");
        write_u16(&mut buf, 0x44, 0x8664);
        write_u16(&mut buf, 0x46, 1);
        write_u16(&mut buf, 0x54, 240);
        write_u16(&mut buf, 0x56, 0x0222);
        write_u16(&mut buf, opt_start, 0x020B);
        write_u32(
            &mut buf,
            opt_start.checked_add(32).expect("section align offset"),
            4096,
        );
        write_u32(
            &mut buf,
            opt_start.checked_add(36).expect("file align offset"),
            file_alignment,
        );
        write_u32(
            &mut buf,
            opt_start.checked_add(60).expect("headers size offset"),
            headers_aligned,
        );
        write_u16(
            &mut buf,
            opt_start.checked_add(68).expect("image version offset"),
            10,
        );
        write_u32(
            &mut buf,
            opt_start.checked_add(108).expect("data dirs offset"),
            16,
        );
        write_bytes(&mut buf, section_start, b".text");
        write_u32(
            &mut buf,
            section_start.checked_add(8).expect("virtual size offset"),
            file_alignment,
        );
        write_u32(
            &mut buf,
            section_start.checked_add(12).expect("virtual addr offset"),
            4096,
        );
        write_u32(
            &mut buf,
            section_start.checked_add(16).expect("raw size offset"),
            file_alignment,
        );
        write_u32(
            &mut buf,
            section_start.checked_add(20).expect("raw ptr offset"),
            headers_aligned,
        );
        write_u32(
            &mut buf,
            section_start
                .checked_add(36)
                .expect("characteristics offset"),
            0x6000_0020,
        );

        buf
    }

    #[test]
    fn parse_rejects_too_small_buffer() {
        // ARRANGE
        let data = [0_u8; 63];

        // ACT
        let result = parse(&data);

        // ASSERT
        assert!(matches!(result, Err(UkiError::InvalidPe(_))));
    }

    #[test]
    fn parse_rejects_invalid_pe() {
        // ARRANGE
        let data = [0_u8; 256];

        // ACT
        let result = parse(&data);

        // ASSERT
        assert!(matches!(result, Err(UkiError::InvalidPe(_))));
    }

    #[test]
    fn parse_parses_valid_pe64() {
        // ARRANGE
        let buf = build_minimal_pe64();

        // ACT
        let meta = parse(&buf).expect("valid PE should parse");

        // ASSERT
        assert_eq!(meta.file_header_offset, 0x44);
        assert_eq!(meta.optional_header_offset, 88);
        assert_eq!(meta.section_alignment, 4096);
        assert_eq!(meta.file_alignment, 512);
        assert!(meta.size_of_headers > 0);
        assert!(meta.existing_section_count >= 1);
        assert!(meta.last_section_file_end >= meta.size_of_headers);
        assert!(meta.last_section_virtual_end >= meta.size_of_headers);
    }

    #[test]
    fn parse_section_extents() {
        // ARRANGE
        let buf = build_minimal_pe64();

        // ACT
        let meta = parse(&buf).expect("valid PE should parse");

        // ASSERT
        assert_eq!(meta.existing_section_count, 1);
        assert!(meta.last_section_file_end > meta.size_of_headers);
        assert!(meta.last_section_virtual_end > meta.size_of_headers);
    }

    #[test]
    fn metadata_has_data_directory_count() {
        // ARRANGE
        let buf = build_minimal_pe64();

        // ACT
        let meta = parse(&buf).expect("valid PE should parse");

        // ASSERT
        assert_eq!(meta.num_data_directories, 16);
    }
}
