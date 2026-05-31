use object::pe;

const DOS_HEADER_SIZE: usize = 64;
const PE_SIGNATURE_SIZE: usize = 4;
const COFF_HEADER_SIZE: usize = 20;
const OPTIONAL_HEADER_SIZE: usize = 240;
const SECTION_HEADER_SIZE: usize = 40;
const FILE_ALIGNMENT: usize = 512;
const SECTION_ALIGNMENT: usize = 4096;
const EXTRA_SECTION_HEADER_SLOTS: usize = 4;

fn write_bytes(buf: &mut [u8], offset: usize, data: &[u8]) {
    let end = offset.saturating_add(data.len());
    buf.get_mut(offset..end)
        .into_iter()
        .for_each(|dst| dst.copy_from_slice(data));
}

fn write_u16(buf: &mut [u8], offset: usize, value: u16) {
    write_bytes(buf, offset, &value.to_le_bytes());
}

fn write_u32(buf: &mut [u8], offset: usize, value: u32) {
    write_bytes(buf, offset, &value.to_le_bytes());
}

fn write_u64(buf: &mut [u8], offset: usize, value: u64) {
    write_bytes(buf, offset, &value.to_le_bytes());
}

fn write_optional_header(
    buf: &mut [u8],
    optional_header_offset: usize,
    num_sections: u16,
    section_size: usize,
) {
    let mut off = optional_header_offset;
    write_u16(buf, off, pe::IMAGE_NT_OPTIONAL_HDR64_MAGIC);
    off = off.saturating_add(2);
    off = off.saturating_add(2);
    write_u32(buf, off, u32::try_from(section_size).unwrap_or_default());
    off = off.saturating_add(4);
    off = off.saturating_add(8);
    write_u32(
        buf,
        off,
        u32::try_from(SECTION_ALIGNMENT).unwrap_or_default(),
    );
    off = off.saturating_add(4);
    write_u32(
        buf,
        off,
        u32::try_from(SECTION_ALIGNMENT).unwrap_or_default(),
    );
    off = off.saturating_add(4);
    write_u64(buf, off, 0x10000);
    off = off.saturating_add(8);
    write_u32(
        buf,
        off,
        u32::try_from(SECTION_ALIGNMENT).unwrap_or_default(),
    );
    off = off.saturating_add(4);
    write_u32(buf, off, u32::try_from(FILE_ALIGNMENT).unwrap_or_default());
    off = off.saturating_add(4);
    off = off.saturating_add(16);
    write_u32(
        buf,
        off,
        u32::try_from(SECTION_ALIGNMENT * 2).unwrap_or_default(),
    );
    off = off.saturating_add(4);

    let headers_aligned = u32::try_from(
        optional_header_offset
            .saturating_add(OPTIONAL_HEADER_SIZE)
            .saturating_add(
                usize::from(num_sections)
                    .saturating_add(EXTRA_SECTION_HEADER_SLOTS)
                    .saturating_mul(SECTION_HEADER_SIZE),
            )
            .next_multiple_of(FILE_ALIGNMENT),
    )
    .unwrap_or_default();
    write_u32(buf, off, headers_aligned);
    off = off.saturating_add(4);
    off = off.saturating_add(4);
    write_u16(buf, off, pe::IMAGE_SUBSYSTEM_EFI_APPLICATION);
    off = off.saturating_add(2);
    off = off.saturating_add(2 + 8 + 8 + 8 + 8 + 4);
    write_u32(buf, off, 0);
}

fn write_text_section_header(buf: &mut [u8], optional_header_offset: usize) {
    let section_headers_offset = optional_header_offset.saturating_add(OPTIONAL_HEADER_SIZE);
    write_bytes(buf, section_headers_offset, b".text");
    write_u32(
        buf,
        section_headers_offset.saturating_add(8),
        u32::try_from(FILE_ALIGNMENT).unwrap_or_default(),
    );
    write_u32(
        buf,
        section_headers_offset.saturating_add(12),
        u32::try_from(SECTION_ALIGNMENT).unwrap_or_default(),
    );
    write_u32(
        buf,
        section_headers_offset.saturating_add(16),
        u32::try_from(FILE_ALIGNMENT).unwrap_or_default(),
    );

    let section_rva = u32::try_from(
        optional_header_offset
            .saturating_add(OPTIONAL_HEADER_SIZE)
            .saturating_add(SECTION_HEADER_SIZE)
            .next_multiple_of(FILE_ALIGNMENT),
    )
    .unwrap_or_default();
    write_u32(buf, section_headers_offset.saturating_add(20), section_rva);
    write_u32(buf, section_headers_offset.saturating_add(36), 0x6000_0020);
}

fn write_extra_section_header(buf: &mut [u8], section_headers_offset: usize, index: usize) {
    let section_header_offset =
        section_headers_offset.saturating_add(index.saturating_mul(SECTION_HEADER_SIZE));
    write_bytes(buf, section_header_offset, b".null");
    write_u32(buf, section_header_offset.saturating_add(8), 1);
    let virtual_address = u32::try_from(index.saturating_add(1).saturating_mul(SECTION_ALIGNMENT))
        .unwrap_or_default();
    write_u32(
        buf,
        section_header_offset.saturating_add(12),
        virtual_address,
    );
}

fn write_extra_section_headers(buf: &mut [u8], optional_header_offset: usize, num_sections: u16) {
    let section_headers_offset = optional_header_offset.saturating_add(OPTIONAL_HEADER_SIZE);
    (1..usize::from(num_sections))
        .for_each(|index| write_extra_section_header(buf, section_headers_offset, index));
}

/// Writes the PE signature, COFF header, optional header, and section headers into the provided buffer.
fn write_pe_headers(buf: &mut [u8], coff_offset: usize, num_sections: u16) {
    let mut off = coff_offset;
    write_bytes(buf, off, b"PE\0\0");
    off = off.saturating_add(PE_SIGNATURE_SIZE);

    write_u16(buf, off, pe::IMAGE_FILE_MACHINE_AMD64);
    off = off.saturating_add(2);
    write_u16(buf, off, num_sections);
    off = off.saturating_add(2);
    off = off.saturating_add(12);
    write_u16(
        buf,
        off,
        u16::try_from(OPTIONAL_HEADER_SIZE).unwrap_or_default(),
    );
    off = off.saturating_add(2);
    let characteristics: u16 =
        pe::IMAGE_FILE_EXECUTABLE_IMAGE | pe::IMAGE_FILE_LARGE_ADDRESS_AWARE | pe::IMAGE_FILE_DLL;
    write_u16(buf, off, characteristics);
    off = off.saturating_add(2);

    write_optional_header(buf, off, num_sections, FILE_ALIGNMENT);
    write_text_section_header(buf, off);
    write_extra_section_headers(buf, off, num_sections);
}

/// Generates a minimal valid PE64 EFI stub.
#[must_use]
pub fn generate_minimal_stub() -> Vec<u8> {
    generate_stub_with_section_count(1)
}

/// Generates a minimal valid PE64 EFI stub with `n` sections declared in the COFF header.
#[must_use]
pub fn generate_stub_with_section_count(n: u16) -> Vec<u8> {
    let section_table_size = SECTION_HEADER_SIZE
        .saturating_mul(usize::from(n).saturating_add(EXTRA_SECTION_HEADER_SLOTS));
    let headers_raw = DOS_HEADER_SIZE
        .saturating_add(PE_SIGNATURE_SIZE)
        .saturating_add(COFF_HEADER_SIZE)
        .saturating_add(OPTIONAL_HEADER_SIZE)
        .saturating_add(section_table_size);
    let headers_aligned = headers_raw.next_multiple_of(FILE_ALIGNMENT);
    let total_size = headers_aligned.saturating_add(FILE_ALIGNMENT);

    let mut pe = vec![0_u8; total_size];
    write_bytes(&mut pe, 0, b"MZ");
    write_u32(
        &mut pe,
        0x3C,
        u32::try_from(DOS_HEADER_SIZE).unwrap_or_default(),
    );

    write_pe_headers(&mut pe, DOS_HEADER_SIZE, n);
    write_bytes(&mut pe, headers_aligned, &[0xC3]);

    pe
}
