//! Authenticode hash calculation for PE files

use std::mem::{offset_of, size_of};

use object::LittleEndian as LE;
use object::pe::{
    IMAGE_DIRECTORY_ENTRY_SECURITY, IMAGE_NT_OPTIONAL_HDR64_MAGIC, ImageDataDirectory,
    ImageFileHeader, ImageOptionalHeader64,
};
use object::read::pe::PeFile64;
use ring::digest::{Context, SHA256};

use crate::{Error, Result};

/// Compute the Authenticode hash (SHA-256) of a PE file
pub fn compute_hash(pe_data: &[u8]) -> Result<[u8; 32]> {
    let pe = PeFile64::parse(pe_data)
        .map_err(|_| Error::PeOperation("invalid or unsupported PE file".into()))?;

    if pe.nt_headers().optional_header.magic.get(LE) != IMAGE_NT_OPTIONAL_HDR64_MAGIC {
        return Err(Error::PeOperation(
            "only PE32+ (64-bit) is supported".into(),
        ));
    }

    let opt = &pe.nt_headers().optional_header;
    let num_dd = opt.number_of_rva_and_sizes.get(LE) as usize;
    if num_dd <= IMAGE_DIRECTORY_ENTRY_SECURITY {
        return Err(Error::PeOperation(
            "no certificate table data directory".into(),
        ));
    }

    let pe_offset = pe.dos_header().nt_headers_offset() as usize;
    let opt_offset = pe_offset + 4 + size_of::<ImageFileHeader>();

    let checksum_offset = opt_offset + offset_of!(ImageOptionalHeader64, check_sum);
    let cert_table_dd_offset = opt_offset
        + size_of::<ImageOptionalHeader64>()
        + IMAGE_DIRECTORY_ENTRY_SECURITY * size_of::<ImageDataDirectory>();

    let cert_table_addr = read_u32_le(pe_data, cert_table_dd_offset)? as usize;
    let cert_table_size = read_u32_le(pe_data, cert_table_dd_offset + 4)? as usize;

    let headers_end = opt.size_of_headers.get(LE) as usize;

    let sections = pe.section_table();
    let mut section_ranges: Vec<(usize, usize)> = sections
        .iter()
        .filter_map(|s| {
            let ptr = s.pointer_to_raw_data.get(LE) as usize;
            let size = s.size_of_raw_data.get(LE) as usize;
            (ptr > 0 && size > 0).then_some((ptr, size))
        })
        .collect();
    section_ranges.sort_by_key(|s| s.0);

    let mut ctx = Context::new(&SHA256);

    hash_range_excluding(
        &mut ctx,
        pe_data,
        0,
        headers_end,
        &[
            (checksum_offset, size_of::<u32>()),
            (cert_table_dd_offset, size_of::<ImageDataDirectory>()),
        ],
    )?;

    for (raw_ptr, raw_size) in &section_ranges {
        let section_end = raw_ptr + raw_size;
        if section_end > pe_data.len() {
            return Err(Error::PeOperation("section extends beyond file".into()));
        }

        if hash_section_excluding_cert(
            &mut ctx,
            pe_data,
            *raw_ptr,
            section_end,
            cert_table_addr,
            cert_table_size,
        ) {
            continue;
        }
        ctx.update(&pe_data[*raw_ptr..section_end]);
    }

    let sections_end = section_ranges
        .iter()
        .map(|(ptr, size)| ptr + size)
        .max()
        .unwrap_or(headers_end);

    let hash_end = if cert_table_addr > 0 && cert_table_size > 0 {
        cert_table_addr.min(pe_data.len())
    } else {
        pe_data.len()
    };

    if sections_end < hash_end {
        ctx.update(&pe_data[sections_end..hash_end]);
    }

    let digest = ctx.finish();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(digest.as_ref());

    Ok(hash)
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
    let cert_end = cert_addr + cert_size;
    if raw_ptr >= cert_end || section_end <= cert_addr {
        return false;
    }
    if raw_ptr < cert_addr {
        ctx.update(&data[raw_ptr..cert_addr]);
    }
    if section_end > cert_end {
        ctx.update(&data[cert_end..section_end]);
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
        .filter(|(off, _)| *off >= start && *off < end)
        .copied()
        .collect();
    exclusions.sort_by_key(|(off, _)| *off);

    let mut pos = start;
    for (excl_off, excl_len) in exclusions {
        if pos < excl_off {
            ctx.update(&data[pos..excl_off]);
        }
        pos = excl_off + excl_len;
    }

    if pos < end {
        ctx.update(&data[pos..end]);
    }

    Ok(())
}

fn read_u32_le(data: &[u8], offset: usize) -> Result<u32> {
    data.get(offset..offset + 4)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| Error::PeOperation("read beyond buffer".into()))
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
}
