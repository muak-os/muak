//! Authenticode hash calculation for PE files

use ring::digest::{Context, SHA256};

use crate::pe::PE32_PLUS_MAGIC;
use crate::{Error, Result};

const PE_SIG_OFFSET_LOCATION: usize = 0x3c;
const PE_SIGNATURE: [u8; 4] = [0x50, 0x45, 0x00, 0x00];
const COFF_HEADER_SIZE: usize = 20;
const CHECKSUM_OFFSET_IN_OPT: usize = 64;
const CHECKSUM_SIZE: usize = 4;
const CERT_TABLE_DD_INDEX: usize = 4;
const DD_ENTRY_SIZE: usize = 8;

/// Compute the Authenticode hash (SHA-256) of a PE file
pub fn compute_hash(pe_data: &[u8]) -> Result<[u8; 32]> {
    if pe_data.len() < 64 {
        return Err(Error::PeOperation("file too small for DOS header".into()));
    }

    let pe_offset = read_u32_le(pe_data, PE_SIG_OFFSET_LOCATION)? as usize;

    if pe_data.len() < pe_offset + 4 {
        return Err(Error::PeOperation("file too small for PE signature".into()));
    }
    if pe_data[pe_offset..pe_offset + 4] != PE_SIGNATURE {
        return Err(Error::PeOperation("invalid PE signature".into()));
    }

    let coff_offset = pe_offset + 4;
    if pe_data.len() < coff_offset + COFF_HEADER_SIZE {
        return Err(Error::PeOperation("file too small for COFF header".into()));
    }

    let num_sections = read_u16_le(pe_data, coff_offset + 2)?;
    let opt_header_size = read_u16_le(pe_data, coff_offset + 16)? as usize;

    let opt_offset = coff_offset + COFF_HEADER_SIZE;
    if pe_data.len() < opt_offset + opt_header_size {
        return Err(Error::PeOperation(
            "file too small for optional header".into(),
        ));
    }

    let magic = read_u16_le(pe_data, opt_offset)?;
    if magic != PE32_PLUS_MAGIC {
        return Err(Error::PeOperation(
            "only PE32+ (64-bit) is supported".into(),
        ));
    }

    let checksum_offset = opt_offset + CHECKSUM_OFFSET_IN_OPT;

    let num_dd_entries = read_u32_le(pe_data, opt_offset + 108)? as usize;
    if num_dd_entries <= CERT_TABLE_DD_INDEX {
        return Err(Error::PeOperation(
            "no certificate table data directory".into(),
        ));
    }

    let dd_offset = opt_offset + 112;
    let cert_table_dd_offset = dd_offset + (CERT_TABLE_DD_INDEX * DD_ENTRY_SIZE);

    let cert_table_addr = read_u32_le(pe_data, cert_table_dd_offset)? as usize;
    let cert_table_size = read_u32_le(pe_data, cert_table_dd_offset + 4)? as usize;

    let sections_offset = opt_offset + opt_header_size;

    let mut sections = Vec::with_capacity(num_sections as usize);
    for i in 0..num_sections as usize {
        let section_offset = sections_offset + (i * 40);
        if pe_data.len() < section_offset + 40 {
            return Err(Error::PeOperation(
                "file too small for section header".into(),
            ));
        }
        let raw_ptr = read_u32_le(pe_data, section_offset + 20)? as usize;
        let raw_size = read_u32_le(pe_data, section_offset + 16)? as usize;
        if raw_ptr > 0 && raw_size > 0 {
            sections.push((raw_ptr, raw_size));
        }
    }
    sections.sort_by_key(|s| s.0);

    let headers_end = read_u32_le(pe_data, opt_offset + 60)? as usize;

    let mut ctx = Context::new(&SHA256);

    hash_range_excluding(
        &mut ctx,
        pe_data,
        0,
        headers_end,
        &[
            (checksum_offset, CHECKSUM_SIZE),
            (cert_table_dd_offset, DD_ENTRY_SIZE),
        ],
    )?;

    for (raw_ptr, raw_size) in &sections {
        let section_end = raw_ptr + raw_size;
        if section_end > pe_data.len() {
            return Err(Error::PeOperation("section extends beyond file".into()));
        }

        if cert_table_addr > 0 && cert_table_size > 0 {
            let cert_end = cert_table_addr + cert_table_size;
            if *raw_ptr < cert_end && section_end > cert_table_addr {
                if *raw_ptr < cert_table_addr {
                    ctx.update(&pe_data[*raw_ptr..cert_table_addr]);
                }
                if section_end > cert_end {
                    ctx.update(&pe_data[cert_end..section_end]);
                }
                continue;
            }
        }
        ctx.update(&pe_data[*raw_ptr..section_end]);
    }

    let sections_end = sections
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

/// Read a little-endian u16 from the buffer
fn read_u16_le(data: &[u8], offset: usize) -> Result<u16> {
    if offset + 2 > data.len() {
        return Err(Error::PeOperation("read beyond buffer".into()));
    }
    Ok(u16::from_le_bytes([data[offset], data[offset + 1]]))
}

/// Read a little-endian u32 from the buffer
fn read_u32_le(data: &[u8], offset: usize) -> Result<u32> {
    if offset + 4 > data.len() {
        return Err(Error::PeOperation("read beyond buffer".into()));
    }
    Ok(u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a little-endian u16 into a buffer at the given offset
    fn put_u16(buf: &mut Vec<u8>, offset: usize, val: u16) {
        buf[offset..offset + 2].copy_from_slice(&val.to_le_bytes());
    }

    /// Write a little-endian u32 into a buffer at the given offset
    fn put_u32(buf: &mut Vec<u8>, offset: usize, val: u32) {
        buf[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
    }

    /// Build a minimal PE32+ binary for testing
    fn build_test_pe(file_alignment: u32, section_raw_offset: u32) -> Vec<u8> {
        let pe_offset: u32 = 0x40; // PE signature right after DOS header

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
        pe[pe_offset as usize..pe_offset as usize + 4].copy_from_slice(&PE_SIGNATURE);

        // -- COFF header --
        // Machine (offset +0): x86-64
        put_u16(&mut pe, coff_offset, 0x8664);
        // NumberOfSections (offset +2)
        put_u16(&mut pe, coff_offset + 2, num_sections);
        // SizeOfOptionalHeader (offset +16)
        put_u16(&mut pe, coff_offset + 16, opt_header_size);

        // -- Optional header (PE32+) --
        // Magic
        put_u16(&mut pe, opt_offset, PE32_PLUS_MAGIC);
        // SizeOfHeaders (offset 60 in optional header)
        put_u32(&mut pe, opt_offset + 60, size_of_headers);
        // NumberOfRvaAndSizes (offset 108)
        put_u32(&mut pe, opt_offset + 108, 16);
        // DD[4] (Certificate Table) left as zero (no existing cert)

        // -- Section header #0 --
        // Name (8 bytes)
        pe[sections_offset..sections_offset + 6].copy_from_slice(b".text\0");
        // SizeOfRawData (offset +16 in section header)
        put_u32(&mut pe, sections_offset + 16, section_size);
        // PointerToRawData (offset +20 in section header)
        put_u32(&mut pe, sections_offset + 20, section_raw_offset);

        // -- Section data --
        pe[section_raw_offset as usize..section_raw_offset as usize + section_data.len()]
            .copy_from_slice(&section_data);

        pe
    }

    #[test]
    fn test_compute_hash_minimal_pe() {
        let pe = build_test_pe(0x200, 0x200);
        let hash = compute_hash(&pe).expect("compute_hash should succeed");
        assert_eq!(hash.len(), 32);

        let hash2 = compute_hash(&pe).expect("second call should succeed");
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_headers_end_uses_size_of_headers() {
        let section_offset: u32 = 0x400;

        let pe_a = build_test_pe(0x200, section_offset);
        let pe_b = build_test_pe(0x400, section_offset);

        let hash_a = compute_hash(&pe_a).expect("hash A");
        let hash_b = compute_hash(&pe_b).expect("hash B");

        assert_ne!(
            hash_a, hash_b,
            "hashes must differ when SizeOfHeaders differs"
        );
    }
}
