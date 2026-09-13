//! Shared PE image fixtures for integration tests.

const FILE_ALIGN: usize = 0x200;
const NT_OFFSET: usize = 0x40;

/// Builds a minimal valid PE32+ image with no sections.
pub fn build_test_pe() -> Vec<u8> {
    let nt_off = NT_OFFSET;
    let file_hdr = nt_off.checked_add(4).expect("file header");
    let opt_off = file_hdr.checked_add(20).expect("opt header");

    let hdr_size = FILE_ALIGN;
    let mut data = vec![0_u8; hdr_size];

    write_bytes(&mut data, 0, b"MZ");
    write_u32(&mut data, 0x3C, u32::try_from(nt_off).expect("nt offset"));
    write_bytes(&mut data, nt_off, b"PE");

    write_u16(&mut data, file_hdr, 0x8664);
    write_u16(
        &mut data,
        file_hdr.checked_add(16).expect("size of opt hdr"),
        0xF0,
    );
    write_u16(
        &mut data,
        file_hdr.checked_add(18).expect("characteristics"),
        0x0002,
    );

    write_u16(&mut data, opt_off, 0x020B);
    write_u32(
        &mut data,
        opt_off.checked_add(16).expect("entry point"),
        u32::try_from(FILE_ALIGN).expect("file align"),
    );
    write_u64(
        &mut data,
        opt_off.checked_add(24).expect("image base"),
        0x0000_0000_0400_0000,
    );
    write_u32(
        &mut data,
        opt_off.checked_add(32).expect("section align"),
        u32::try_from(FILE_ALIGN).expect("section align"),
    );
    write_u32(
        &mut data,
        opt_off.checked_add(36).expect("file align"),
        u32::try_from(FILE_ALIGN).expect("file align 2"),
    );
    write_u16(
        &mut data,
        opt_off.checked_add(44).expect("major image version"),
        1,
    );
    write_u32(
        &mut data,
        opt_off.checked_add(56).expect("size of image"),
        u32::try_from(FILE_ALIGN).expect("image size"),
    );
    write_u32(
        &mut data,
        opt_off.checked_add(60).expect("size of headers"),
        u32::try_from(FILE_ALIGN).expect("headers size"),
    );
    write_u32(&mut data, opt_off.checked_add(108).expect("data dirs"), 16);

    data
}

/// Appends a section with `name` and `content` to a fixture image.
pub fn add_section(data: &mut Vec<u8>, name: [u8; 8], content: &[u8]) {
    let nt_off = NT_OFFSET;
    let file_hdr = nt_off.checked_add(4).expect("file header");
    let opt_off = file_hdr.checked_add(20).expect("opt header");
    let dd_off = opt_off.checked_add(112).expect("data dirs");
    let shdr_off = dd_off.checked_add(16 * 8).expect("section headers");

    let count_off = file_hdr.checked_add(2).expect("count field");
    let count_end = count_off.checked_add(2).expect("count end");
    let section_index = {
        let chunk = data
            .get(count_off..count_end)
            .and_then(|slice| <[u8; 2]>::try_from(slice).ok())
            .unwrap_or([0; 2]);
        u16::from_le_bytes(chunk)
    };

    let index_usize = usize::from(section_index);
    let raw_offset = FILE_ALIGN
        .checked_mul(index_usize.checked_add(1).expect("index + 1"))
        .expect("raw offset");
    let raw_size = content.len().next_multiple_of(FILE_ALIGN);

    let needed = raw_offset.checked_add(raw_size).expect("needed size");
    data.resize(data.len().max(needed), 0);

    let content_end = raw_offset.checked_add(content.len()).expect("content end");
    data.get_mut(raw_offset..content_end)
        .expect("section content range")
        .copy_from_slice(content);

    let section_header = shdr_off
        .checked_add(index_usize * 40)
        .expect("section header");
    let content_len = u32::try_from(content.len()).expect("content len");
    let raw_offset_u32 = u32::try_from(raw_offset).expect("raw offset");
    let raw_size_u32 = u32::try_from(raw_size).expect("raw size");

    data.get_mut(section_header..section_header.checked_add(8).expect("name range"))
        .expect("section header name range")
        .copy_from_slice(&name);
    write_u32(
        data,
        section_header.checked_add(8).expect("vs"),
        content_len,
    );
    write_u32(
        data,
        section_header.checked_add(12).expect("va"),
        raw_offset_u32,
    );
    write_u32(
        data,
        section_header.checked_add(16).expect("rs"),
        raw_size_u32,
    );
    write_u32(
        data,
        section_header.checked_add(20).expect("ptr"),
        raw_offset_u32,
    );

    let new_count = section_index.checked_add(1).expect("section count");
    write_u16(
        data,
        file_hdr.checked_add(2).expect("count field"),
        new_count,
    );
    write_u32(
        data,
        opt_off.checked_add(56).expect("size of image field"),
        u32::try_from(raw_offset.checked_add(raw_size).expect("image size")).expect("image size"),
    );
}

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

fn write_u64(buf: &mut [u8], offset: usize, value: u64) {
    write_bytes(buf, offset, &value.to_le_bytes());
}
