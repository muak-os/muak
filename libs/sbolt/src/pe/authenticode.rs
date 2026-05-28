//! Authenticode hash calculation for PE files.

use core::mem::{offset_of, size_of};

use object::LittleEndian as LE;
use object::pe::{
    IMAGE_DIRECTORY_ENTRY_SECURITY, ImageDataDirectory, ImageFileHeader, ImageOptionalHeader64,
};
use object::read::pe::PeFile64;
use ring::digest::{Context, SHA256};

use crate::error::{Result, SboltError};

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
        .map_err(|_parse_error| SboltError::PeOperation("invalid or unsupported PE file".into()))?;

    let opt = &pe.nt_headers().optional_header;
    let num_dd = read_directory_count(opt)?;
    if num_dd <= IMAGE_DIRECTORY_ENTRY_SECURITY {
        return Err(SboltError::PeOperation(
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
            return Err(SboltError::PeOperation(
                "section extends beyond file".into(),
            ));
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
            .ok_or_else(|| SboltError::PeOperation("section extends beyond file".into()))?;
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
            .ok_or_else(|| SboltError::PeOperation("trailing hash range exceeds file".into()))?;
        ctx.update(trailing_bytes);
    }

    let digest = ctx.finish();
    let mut hash = [0_u8; 32];
    hash.copy_from_slice(digest.as_ref());

    Ok(hash)
}

fn read_directory_count(opt: &ImageOptionalHeader64) -> Result<usize> {
    u32_to_usize(opt.number_of_rva_and_sizes.get(LE), "directory count")
}

fn build_hash_metadata(
    pe: &PeFile64<'_>,
    pe_data: &[u8],
    opt: &ImageOptionalHeader64,
) -> Result<PeHashMetadata> {
    let pe_offset = u32_to_usize(pe.dos_header().nt_headers_offset(), "PE header offset")?;
    let opt_offset = checked_add(
        pe_offset,
        PE_SIGNATURE_PREFIX_SIZE,
        "optional header offset",
    )?;
    let opt_offset = checked_add(
        opt_offset,
        size_of::<ImageFileHeader>(),
        "optional header offset",
    )?;
    let checksum_offset = checked_add(
        opt_offset,
        offset_of!(ImageOptionalHeader64, check_sum),
        "checksum field offset",
    )?;
    let cert_table_index_offset = checked_mul(
        IMAGE_DIRECTORY_ENTRY_SECURITY,
        size_of::<ImageDataDirectory>(),
        "certificate table directory offset",
    )?;
    let cert_table_relative_offset = checked_add(
        size_of::<ImageOptionalHeader64>(),
        cert_table_index_offset,
        "certificate table directory offset",
    )?;
    let cert_table_dd_offset = checked_add(
        opt_offset,
        cert_table_relative_offset,
        "certificate table offset",
    )?;
    let cert_table_addr = u32_to_usize(
        read_u32_le(pe_data, cert_table_dd_offset)?,
        "certificate table address",
    )?;
    let cert_table_size_offset = checked_add(
        cert_table_dd_offset,
        CERT_TABLE_ENTRY_SIZE,
        "certificate table size offset",
    )?;
    let cert_table_size = read_u32_le(pe_data, cert_table_size_offset)?;
    let cert_table_size = u32_to_usize(cert_table_size, "certificate table size")?;
    let headers_end = u32_to_usize(opt.size_of_headers.get(LE), "headers size")?;

    Ok(PeHashMetadata {
        checksum_offset,
        cert_table_dd_offset,
        cert_table_addr,
        cert_table_size,
        headers_end,
        section_ranges: collect_section_ranges(pe)?,
    })
}

fn collect_section_ranges(pe: &PeFile64<'_>) -> Result<Vec<(usize, usize)>> {
    let sections = pe.section_table();
    let mut section_ranges = Vec::with_capacity(sections.len());
    for section in sections.iter() {
        let ptr = u32_to_usize(section.pointer_to_raw_data.get(LE), "section raw pointer")?;
        let size = u32_to_usize(section.size_of_raw_data.get(LE), "section raw size")?;
        if ptr > 0 && size > 0 {
            section_ranges.push((ptr, size));
        }
    }
    section_ranges.sort_by_key(|section_range| section_range.0);

    Ok(section_ranges)
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
                .ok_or_else(|| SboltError::PeOperation("excluded range exceeds file".into()))?;
            ctx.update(range_bytes);
        }
        pos = checked_add(excl_off, excl_len, "excluded range end")?;
    }

    if pos < end {
        let range_bytes = data
            .get(pos..end)
            .ok_or_else(|| SboltError::PeOperation("hashed range exceeds file".into()))?;
        ctx.update(range_bytes);
    }

    Ok(())
}

fn read_u32_le(data: &[u8], offset: usize) -> Result<u32> {
    let end = checked_add(offset, CERT_TABLE_ENTRY_SIZE, "read_u32 range end")?;

    data.get(offset..end)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| SboltError::PeOperation("read beyond buffer".into()))
}

fn checked_add(lhs: usize, rhs: usize, context: &str) -> Result<usize> {
    lhs.checked_add(rhs)
        .ok_or_else(|| SboltError::PeOperation(format!("{context} overflow")))
}

fn checked_mul(lhs: usize, rhs: usize, context: &str) -> Result<usize> {
    lhs.checked_mul(rhs)
        .ok_or_else(|| SboltError::PeOperation(format!("{context} overflow")))
}

fn u32_to_usize(value: u32, context: &str) -> Result<usize> {
    usize::try_from(value)
        .map_err(|_conversion_error| SboltError::PeOperation(format!("{context} exceeds usize")))
}

#[cfg(test)]
mod tests {
    use object::pe::IMAGE_NT_OPTIONAL_HDR64_MAGIC;
    use object::read::pe::PeFile64;

    use super::*;

    fn put_u16(buf: &mut Vec<u8>, offset: usize, val: u16) {
        buf[offset..offset + 2].copy_from_slice(&val.to_le_bytes());
    }

    fn put_u32(buf: &mut Vec<u8>, offset: usize, val: u32) {
        buf[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
    }

    struct TestPeConfig {
        file_alignment: u32,
        section_raw_offsets: Vec<u32>,
        section_size: u32,
        cert_table_addr: u32,
        cert_table_size: u32,
        trailing_size: usize,
    }

    impl TestPeConfig {
        fn single_section(file_alignment: u32, section_raw_offset: u32) -> Self {
            Self {
                file_alignment,
                section_raw_offsets: vec![section_raw_offset],
                section_size: 16,
                cert_table_addr: 0,
                cert_table_size: 0,
                trailing_size: 0,
            }
        }
    }

    /// Build a minimal PE32+ binary for testing.
    fn build_test_pe(file_alignment: u32, section_raw_offset: u32) -> Vec<u8> {
        build_test_pe_with_config(&TestPeConfig::single_section(
            file_alignment,
            section_raw_offset,
        ))
    }

    fn build_test_pe_with_config(config: &TestPeConfig) -> Vec<u8> {
        let pe_offset: u32 = 0x40;

        // Layout:
        //   0x00  DOS header (64 bytes), PE offset at 0x3C
        //   0x40  PE signature (4 bytes)
        //   0x44  COFF header (20 bytes)
        //   0x58  Optional header: PE32+ (240 bytes = 112 + 16*8)
        let coff_offset = pe_offset as usize + 4;
        let opt_offset = coff_offset + 20;
        let opt_header_size: u16 = 240; // 112 fixed + 16 data dirs * 8
        let num_sections: u16 = config
            .section_raw_offsets
            .len()
            .try_into()
            .expect("test section count fits u16");
        let sections_offset = opt_offset + opt_header_size as usize;
        let headers_raw_end = sections_offset + 40 * usize::from(num_sections);

        // SizeOfHeaders = headers_raw_end rounded up to file_alignment
        let size_of_headers = ((headers_raw_end as u32 + config.file_alignment - 1)
            / config.file_alignment)
            * config.file_alignment;

        // Section data
        let section_size = config.section_size;

        let sections_end = config
            .section_raw_offsets
            .iter()
            .map(|offset| *offset as usize + section_size as usize)
            .max()
            .unwrap_or(size_of_headers as usize);
        let cert_end = config.cert_table_addr as usize + config.cert_table_size as usize;
        let total_size = sections_end.max(cert_end) + config.trailing_size;
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
        put_u32(
            &mut pe,
            opt_offset + 112 + IMAGE_DIRECTORY_ENTRY_SECURITY * 8,
            config.cert_table_addr,
        );
        put_u32(
            &mut pe,
            opt_offset + 112 + IMAGE_DIRECTORY_ENTRY_SECURITY * 8 + 4,
            config.cert_table_size,
        );

        for (index, section_raw_offset) in config.section_raw_offsets.iter().enumerate() {
            let section_offset = sections_offset + index * 40;
            pe[section_offset..section_offset + 6].copy_from_slice(b".text\0");
            put_u32(&mut pe, section_offset + 16, section_size);
            put_u32(&mut pe, section_offset + 20, *section_raw_offset);

            let data_start = *section_raw_offset as usize;
            let data_end = data_start + section_size as usize;
            pe[data_start..data_end].fill(0xde_u8.wrapping_add(index as u8));
        }

        if config.cert_table_size > 0 {
            let cert_start = config.cert_table_addr as usize;
            let cert_end = cert_start + config.cert_table_size as usize;
            pe[cert_start..cert_end].fill(0x5a);
        }

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
    fn compute_hash_includes_trailing_data_before_certificate_table() {
        // ARRANGE
        let mut pe = build_test_pe_with_config(&TestPeConfig {
            file_alignment: 0x200,
            section_raw_offsets: vec![0x200],
            section_size: 16,
            cert_table_addr: 0x240,
            cert_table_size: 16,
            trailing_size: 0,
        });
        pe[0x220..0x230].fill(0xa5);
        let mut changed = pe.clone();
        changed[0x220] ^= 0xff;

        // ACT
        let original_hash = compute_hash(&pe).expect("hash original");
        let changed_hash = compute_hash(&changed).expect("hash changed");

        // ASSERT
        assert_ne!(original_hash, changed_hash);
    }

    #[test]
    fn compute_hash_ignores_certificate_table_contents() {
        // ARRANGE
        let pe = build_test_pe_with_config(&TestPeConfig {
            file_alignment: 0x200,
            section_raw_offsets: vec![0x200],
            section_size: 16,
            cert_table_addr: 0x220,
            cert_table_size: 16,
            trailing_size: 0,
        });
        let mut changed = pe.clone();
        changed[0x220] ^= 0xff;

        // ACT
        let original_hash = compute_hash(&pe).expect("hash original");
        let changed_hash = compute_hash(&changed).expect("hash changed");

        // ASSERT
        assert_eq!(original_hash, changed_hash);
    }

    #[test]
    fn compute_hash_handles_empty_section_table() {
        // ARRANGE
        let mut pe = build_test_pe(0x200, 0x200);
        put_u16(&mut pe, 0x44 + 2, 0);

        // ACT
        let hash = compute_hash(&pe).expect("hash no-section PE");

        // ASSERT
        assert_ne!(hash, [0_u8; 32]);
    }

    #[test]
    fn compute_hash_skips_certificate_table_inside_section() {
        // ARRANGE
        let pe = build_test_pe_with_config(&TestPeConfig {
            file_alignment: 0x200,
            section_raw_offsets: vec![0x200],
            section_size: 32,
            cert_table_addr: 0x208,
            cert_table_size: 8,
            trailing_size: 0,
        });
        let mut changed = pe.clone();
        changed[0x208] ^= 0xff;

        // ACT
        let original_hash = compute_hash(&pe).expect("hash original");
        let changed_hash = compute_hash(&changed).expect("hash changed");

        // ASSERT
        assert_eq!(original_hash, changed_hash);
    }

    #[test]
    fn metadata_helpers_read_directory_and_sorted_sections() {
        // ARRANGE
        let pe_data = build_test_pe_with_config(&TestPeConfig {
            file_alignment: 0x200,
            section_raw_offsets: vec![0x300, 0x200],
            section_size: 16,
            cert_table_addr: 0x400,
            cert_table_size: 8,
            trailing_size: 0,
        });
        let pe = PeFile64::parse(pe_data.as_slice()).expect("parse test PE");
        let opt = &pe.nt_headers().optional_header;

        // ACT
        let directory_count = read_directory_count(opt).expect("read directory count");
        let metadata = build_hash_metadata(&pe, &pe_data, opt).expect("build hash metadata");

        // ASSERT
        assert_eq!(directory_count, 16);
        assert_eq!(metadata.cert_table_addr, 0x400);
        assert_eq!(metadata.cert_table_size, 8);
        assert_eq!(metadata.section_ranges, vec![(0x200, 16), (0x300, 16)]);
    }

    #[test]
    fn build_hash_metadata_rejects_truncated_certificate_directory() {
        // ARRANGE
        let pe_data = build_test_pe(0x200, 0x200);
        let pe = PeFile64::parse(pe_data.as_slice()).expect("parse test PE");
        let opt = &pe.nt_headers().optional_header;
        let truncated_len = 0x58 + 112 + IMAGE_DIRECTORY_ENTRY_SECURITY * 8 + 4;

        // ACT
        let result = build_hash_metadata(&pe, &pe_data[..truncated_len], opt);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn read_u32_le_reads_expected_value() {
        // ARRANGE
        let data = [0, 0xef, 0xbe, 0xad, 0xde];

        // ACT
        let value = read_u32_le(&data, 1).expect("read u32");

        // ASSERT
        assert_eq!(value, 0xdead_beef);
    }

    #[test]
    fn compute_hash_rejects_invalid_pe_bytes() {
        // ARRANGE
        let data = b"not-a-pe-file";

        // ACT
        let result = compute_hash(data);

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
    fn hash_section_excluding_cert_returns_false_for_non_overlaps() {
        // ARRANGE
        let data = b"0123456789abcdef";
        let mut no_cert_ctx = Context::new(&SHA256);
        let mut zero_size_ctx = Context::new(&SHA256);
        let mut before_ctx = Context::new(&SHA256);
        let mut after_ctx = Context::new(&SHA256);

        // ACT
        let no_cert = hash_section_excluding_cert(&mut no_cert_ctx, data, 0, 4, 0, 4);
        let zero_size = hash_section_excluding_cert(&mut zero_size_ctx, data, 0, 4, 4, 0);
        let before = hash_section_excluding_cert(&mut before_ctx, data, 0, 4, 8, 4);
        let after = hash_section_excluding_cert(&mut after_ctx, data, 8, 12, 0, 4);

        // ASSERT
        assert!(!no_cert);
        assert!(!zero_size);
        assert!(!before);
        assert!(!after);
    }

    #[test]
    fn hash_section_excluding_cert_returns_false_for_invalid_ranges() {
        // ARRANGE
        let data = b"0123456789abcdef";
        let mut overflowing_ctx = Context::new(&SHA256);
        let mut prefix_ctx = Context::new(&SHA256);
        let mut suffix_ctx = Context::new(&SHA256);

        // ACT
        let overflowing =
            hash_section_excluding_cert(&mut overflowing_ctx, data, 0, data.len(), usize::MAX, 1);
        let bad_prefix = hash_section_excluding_cert(&mut prefix_ctx, data, 0, data.len(), 20, 4);
        let bad_suffix = hash_section_excluding_cert(&mut suffix_ctx, data, 8, 20, 4, 4);

        // ASSERT
        assert!(!overflowing);
        assert!(!bad_prefix);
        assert!(!bad_suffix);
    }

    #[test]
    fn hash_range_excluding_hashes_prefix_middle_and_suffix() {
        // ARRANGE
        let data = b"0123456789abcdef";
        let mut actual = Context::new(&SHA256);
        let mut expected = Context::new(&SHA256);
        expected.update(&data[0..2]);
        expected.update(&data[4..8]);
        expected.update(&data[10..data.len()]);

        // ACT
        hash_range_excluding(&mut actual, data, 0, data.len(), &[(8, 2), (2, 2)])
            .expect("hash range excluding ranges");

        // ASSERT
        assert_eq!(actual.finish().as_ref(), expected.finish().as_ref());
    }

    #[test]
    fn hash_range_excluding_rejects_end_beyond_file() {
        // ARRANGE
        let mut ctx = Context::new(&SHA256);

        // ACT
        let result = hash_range_excluding(&mut ctx, b"abc", 0, 4, &[]);

        // ASSERT
        assert!(result.is_err());
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
        let conversion_result = u32_to_usize(0, "conversion");
        let read_result = read_u32_le(&[1_u8, 2, 3], 0);

        // ASSERT
        assert!(add_result.is_err());
        assert!(mul_result.is_err());
        assert_eq!(conversion_result.expect("convert u32"), 0);
        assert!(read_result.is_err());
    }
}
