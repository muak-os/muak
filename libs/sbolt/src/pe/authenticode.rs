//! Authenticode hash calculation for PE files.

use core::mem::{offset_of, size_of};

use object::LittleEndian as LE;
use object::pe::{
    IMAGE_DIRECTORY_ENTRY_SECURITY, IMAGE_NT_OPTIONAL_HDR64_MAGIC, ImageDataDirectory,
    ImageFileHeader, ImageOptionalHeader64,
};
use object::read::pe::PeFile64;
use ring::digest::{Context, SHA256};

use crate::Error;
use crate::Result;

const PE_SIGNATURE_PREFIX_SIZE: usize = 4;
const CERT_TABLE_ENTRY_SIZE: usize = 4;

struct PeHashMetadata {
    checksum_offset: usize,
    cert_table_dd_offset: usize,
    cert_table_addr: usize,
    cert_table_size: usize,
    headers_end: usize,
    section_ranges: Vec<(usize, usize)>,
}

/// Compute the Authenticode hash (SHA-256) of a PE file.
///
/// # Errors
///
/// Returns an error if the PE file is malformed or if any hashed range falls
/// outside the file bounds.
pub fn compute_hash(pe_data: &[u8]) -> Result<[u8; 32]> {
    let pe = PeFile64::parse(pe_data)
        .map_err(|_parse_error| Error::PeOperation("invalid or unsupported PE file".into()))?;

    if pe.nt_headers().optional_header.magic.get(LE) != IMAGE_NT_OPTIONAL_HDR64_MAGIC {
        return Err(Error::PeOperation(
            "only PE32+ (64-bit) is supported".into(),
        ));
    }

    let opt = &pe.nt_headers().optional_header;
    let num_dd = read_directory_count(opt)?;
    if num_dd <= IMAGE_DIRECTORY_ENTRY_SECURITY {
        return Err(Error::PeOperation(
            "no certificate table data directory".into(),
        ));
    }
    let metadata = build_hash_metadata(&pe, pe_data, opt)?;

    let mut ctx = Context::new(&SHA256);

    hash_range_excluding(
        &mut ctx,
        pe_data,
        0,
        metadata.headers_end,
        &[
            (metadata.checksum_offset, size_of::<u32>()),
            (
                metadata.cert_table_dd_offset,
                size_of::<ImageDataDirectory>(),
            ),
        ],
    )?;

    for &(raw_ptr, raw_size) in &metadata.section_ranges {
        let section_end = checked_add(raw_ptr, raw_size, "section end")?;
        if section_end > pe_data.len() {
            return Err(Error::PeOperation("section extends beyond file".into()));
        }

        if hash_section_excluding_cert(
            &mut ctx,
            pe_data,
            raw_ptr,
            section_end,
            metadata.cert_table_addr,
            metadata.cert_table_size,
        ) {
            continue;
        }
        let section_bytes = pe_data
            .get(raw_ptr..section_end)
            .ok_or_else(|| Error::PeOperation("section extends beyond file".into()))?;
        ctx.update(section_bytes);
    }

    let sections_end = metadata
        .section_ranges
        .iter()
        .map(|&(ptr, size)| checked_add(ptr, size, "sections end"))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .max()
        .unwrap_or(metadata.headers_end);

    let hash_end = if metadata.cert_table_addr > 0 && metadata.cert_table_size > 0 {
        metadata.cert_table_addr.min(pe_data.len())
    } else {
        pe_data.len()
    };

    if sections_end < hash_end {
        let trailing_bytes = pe_data
            .get(sections_end..hash_end)
            .ok_or_else(|| Error::PeOperation("trailing hash range exceeds file".into()))?;
        ctx.update(trailing_bytes);
    }

    let digest = ctx.finish();
    let mut hash = [0_u8; 32];
    hash.copy_from_slice(digest.as_ref());

    Ok(hash)
}

fn read_directory_count(opt: &ImageOptionalHeader64) -> Result<usize> {
    usize::try_from(opt.number_of_rva_and_sizes.get(LE))
        .map_err(|_directory_count_error| Error::PeOperation("invalid data directory count".into()))
}

fn build_hash_metadata(
    pe: &PeFile64<'_>,
    pe_data: &[u8],
    opt: &ImageOptionalHeader64,
) -> Result<PeHashMetadata> {
    let pe_offset = usize::try_from(pe.dos_header().nt_headers_offset())
        .map_err(|_offset_error| Error::PeOperation("PE header offset exceeds usize".into()))?;
    let opt_offset = checked_add(
        checked_add(
            pe_offset,
            PE_SIGNATURE_PREFIX_SIZE,
            "optional header offset",
        )?,
        size_of::<ImageFileHeader>(),
        "optional header offset",
    )?;
    let checksum_offset = checked_add(
        opt_offset,
        offset_of!(ImageOptionalHeader64, check_sum),
        "checksum field offset",
    )?;
    let cert_table_dd_offset = checked_add(
        opt_offset,
        checked_add(
            size_of::<ImageOptionalHeader64>(),
            checked_mul(
                IMAGE_DIRECTORY_ENTRY_SECURITY,
                size_of::<ImageDataDirectory>(),
                "certificate table directory offset",
            )?,
            "certificate table directory offset",
        )?,
        "certificate table offset",
    )?;
    let cert_table_addr = usize::try_from(read_u32_le(pe_data, cert_table_dd_offset)?).map_err(
        |_cert_addr_error| Error::PeOperation("certificate table offset exceeds usize".into()),
    )?;
    let cert_table_size = usize::try_from(read_u32_le(
        pe_data,
        checked_add(
            cert_table_dd_offset,
            CERT_TABLE_ENTRY_SIZE,
            "certificate table size offset",
        )?,
    )?)
    .map_err(|_cert_size_error| {
        Error::PeOperation("certificate table size exceeds usize".into())
    })?;
    let headers_end = usize::try_from(opt.size_of_headers.get(LE))
        .map_err(|_headers_size_error| Error::PeOperation("header size exceeds usize".into()))?;

    Ok(PeHashMetadata {
        checksum_offset,
        cert_table_dd_offset,
        cert_table_addr,
        cert_table_size,
        headers_end,
        section_ranges: collect_section_ranges(pe),
    })
}

fn collect_section_ranges(pe: &PeFile64<'_>) -> Vec<(usize, usize)> {
    let mut section_ranges: Vec<(usize, usize)> = pe
        .section_table()
        .iter()
        .filter_map(|section| {
            let ptr = usize::try_from(section.pointer_to_raw_data.get(LE)).ok()?;
            let size = usize::try_from(section.size_of_raw_data.get(LE)).ok()?;
            (ptr > 0 && size > 0).then_some((ptr, size))
        })
        .collect();
    section_ranges.sort_by_key(|section_range| section_range.0);
    section_ranges
}

/// Hash a section range, skipping the certificate table if it overlaps.
fn hash_section_excluding_cert(
    ctx: &mut Context,
    data: &[u8],
    raw_ptr: usize,
    section_end: usize,
    cert_addr: usize,
    cert_size: usize,
) -> bool {
    if cert_addr == 0 || cert_size == 0 {
        return false;
    }
    let Some(cert_end) = cert_addr.checked_add(cert_size) else {
        return false;
    };
    if raw_ptr >= cert_end || section_end <= cert_addr {
        return false;
    }
    if raw_ptr < cert_addr {
        let Some(prefix_bytes) = data.get(raw_ptr..cert_addr) else {
            return false;
        };
        ctx.update(prefix_bytes);
    }
    if section_end > cert_end {
        let Some(suffix_bytes) = data.get(cert_end..section_end) else {
            return false;
        };
        ctx.update(suffix_bytes);
    }
    true
}

/// Hash a range of data, excluding specified regions.
fn hash_range_excluding(
    ctx: &mut Context,
    data: &[u8],
    start: usize,
    end: usize,
    exclusions: &[(usize, usize)],
) -> Result<()> {
    let mut exclusions: Vec<_> = exclusions
        .iter()
        .filter(|&&(off, _)| off >= start && off < end)
        .copied()
        .collect();
    exclusions.sort_by_key(|&(off, _)| off);

    let mut pos = start;
    for (excl_off, excl_len) in exclusions {
        if pos < excl_off {
            let range_bytes = data
                .get(pos..excl_off)
                .ok_or_else(|| Error::PeOperation("excluded range exceeds file".into()))?;
            ctx.update(range_bytes);
        }
        pos = checked_add(excl_off, excl_len, "excluded range end")?;
    }

    if pos < end {
        let range_bytes = data
            .get(pos..end)
            .ok_or_else(|| Error::PeOperation("hashed range exceeds file".into()))?;
        ctx.update(range_bytes);
    }

    Ok(())
}

fn read_u32_le(data: &[u8], offset: usize) -> Result<u32> {
    let end = checked_add(offset, CERT_TABLE_ENTRY_SIZE, "read_u32 range end")?;

    data.get(offset..end)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| Error::PeOperation("read beyond buffer".into()))
}

fn checked_add(lhs: usize, rhs: usize, context: &str) -> Result<usize> {
    lhs.checked_add(rhs)
        .ok_or_else(|| Error::PeOperation(format!("{context} overflow")))
}

fn checked_mul(lhs: usize, rhs: usize, context: &str) -> Result<usize> {
    lhs.checked_mul(rhs)
        .ok_or_else(|| Error::PeOperation(format!("{context} overflow")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put_u16(buf: &mut Vec<u8>, offset: usize, val: u16) {
        buf[offset..offset + 2].copy_from_slice(&val.to_le_bytes());
    }

    fn put_u32(buf: &mut Vec<u8>, offset: usize, val: u32) {
        buf[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
    }

    /// Build a minimal PE32+ binary for testing
    fn build_test_pe(file_alignment: u32, section_raw_offset: u32) -> Vec<u8> {
        let pe_offset: u32 = 0x40;

        // Layout:
        //   0x00  DOS header (64 bytes), PE offset at 0x3C
        //   0x40  PE signature (4 bytes)
        //   0x44  COFF header (20 bytes)
        //   0x58  Optional header: PE32+ (240 bytes = 112 + 16*8)
        //   0x148 Section header #0 (40 bytes)
        //   0x170 End of headers (unaligned)
        let coff_offset = pe_offset as usize + 4;
        let opt_offset = coff_offset + 20;
        let opt_header_size: u16 = 240; // 112 fixed + 16 data dirs * 8
        let num_sections: u16 = 1;
        let sections_offset = opt_offset + opt_header_size as usize;
        let headers_raw_end = sections_offset + 40; // one section header

        // SizeOfHeaders = headers_raw_end rounded up to file_alignment
        let size_of_headers =
            ((headers_raw_end as u32 + file_alignment - 1) / file_alignment) * file_alignment;

        // Section data
        let section_data: [u8; 16] = [0xDE; 16];
        let section_size = section_data.len() as u32;

        let total_size = section_raw_offset as usize + section_data.len();
        let mut pe = vec![0u8; total_size];

        // -- DOS header --
        pe[0] = 0x4d; // 'M'
        pe[1] = 0x5a; // 'Z'
        put_u32(&mut pe, 0x3c, pe_offset);

        // -- PE signature --
        pe[pe_offset as usize..pe_offset as usize + 4].copy_from_slice(&[0x50, 0x45, 0x00, 0x00]);

        // -- COFF header --
        put_u16(&mut pe, coff_offset, 0x8664); // Machine: x86-64
        put_u16(&mut pe, coff_offset + 2, num_sections);
        put_u16(&mut pe, coff_offset + 16, opt_header_size);

        // -- Optional header (PE32+) --
        put_u16(&mut pe, opt_offset, IMAGE_NT_OPTIONAL_HDR64_MAGIC);
        put_u32(&mut pe, opt_offset + 60, size_of_headers);
        put_u32(&mut pe, opt_offset + 108, 16); // NumberOfRvaAndSizes

        // -- Section header #0 --
        pe[sections_offset..sections_offset + 6].copy_from_slice(b".text\0");
        put_u32(&mut pe, sections_offset + 16, section_size);
        put_u32(&mut pe, sections_offset + 20, section_raw_offset);

        // -- Section data --
        pe[section_raw_offset as usize..section_raw_offset as usize + section_data.len()]
            .copy_from_slice(&section_data);

        pe
    }

    #[test]
    fn compute_hash_minimal_pe() {
        // ARRANGE
        let pe = build_test_pe(0x200, 0x200);

        // ACT
        let hash = compute_hash(&pe).expect("compute_hash should succeed");
        let hash2 = compute_hash(&pe).expect("second call should succeed");

        // ASSERT
        assert_eq!(hash.len(), 32);
        assert_eq!(hash, hash2);
    }

    #[test]
    fn headers_end_uses_size_of_headers() {
        // ARRANGE
        let section_offset: u32 = 0x400;
        let pe_a = build_test_pe(0x200, section_offset);
        let pe_b = build_test_pe(0x400, section_offset);

        // ACT
        let hash_a = compute_hash(&pe_a).expect("hash A");
        let hash_b = compute_hash(&pe_b).expect("hash B");

        // ASSERT
        assert_ne!(
            hash_a, hash_b,
            "hashes must differ when SizeOfHeaders differs"
        );
    }

    #[test]
    fn compute_hash_rejects_non_pe32_plus_images() {
        // ARRANGE
        let mut pe = build_test_pe(0x200, 0x200);
        put_u16(&mut pe, 0x58, 0x10b);

        // ACT
        let result = compute_hash(&pe);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn compute_hash_rejects_missing_certificate_directory() {
        // ARRANGE
        let mut pe = build_test_pe(0x200, 0x200);
        put_u32(&mut pe, 0x58 + 108, IMAGE_DIRECTORY_ENTRY_SECURITY as u32);

        // ACT
        let result = compute_hash(&pe);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn compute_hash_rejects_section_extending_beyond_file() {
        // ARRANGE
        let mut pe = build_test_pe(0x200, 0x200);
        put_u32(&mut pe, 0x148 + 16, 0x1000);

        // ACT
        let result = compute_hash(&pe);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn hash_section_excluding_cert_hashes_prefix_and_suffix() {
        // ARRANGE
        let data = b"0123456789abcdef";
        let mut ctx = Context::new(&SHA256);

        // ACT
        let overlapped = hash_section_excluding_cert(&mut ctx, data, 0, data.len(), 4, 4);
        let digest = ctx.finish();

        let mut expected = Context::new(&SHA256);
        expected.update(&data[..4]);
        expected.update(&data[8..]);

        // ASSERT
        assert!(overlapped);
        assert_eq!(digest.as_ref(), expected.finish().as_ref());
    }

    #[test]
    fn hash_range_excluding_rejects_overflowing_exclusion() {
        // ARRANGE
        let mut ctx = Context::new(&SHA256);

        // ACT
        let result = hash_range_excluding(&mut ctx, b"abc", 0, 3, &[(2, usize::MAX)]);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn helper_arithmetic_and_reads_validate_bounds() {
        // ACT
        let add_result = checked_add(usize::MAX, 1, "add");
        let mul_result = checked_mul(usize::MAX, 2, "mul");
        let read_result = read_u32_le(&[1_u8, 2, 3], 0);

        // ASSERT
        assert!(add_result.is_err());
        assert!(mul_result.is_err());
        assert!(read_result.is_err());
    }
}
