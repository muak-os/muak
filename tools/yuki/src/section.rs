//! PE section creation and embedding.

use std::mem;

use object::LittleEndian as LE;
use object::pe::ImageSectionHeader;

use crate::YukiError;
use crate::binary::align_to;
use crate::constants;
use crate::pe::PeMetadata;

/// Computed file and virtual memory layout for a set of PE sections to be embedded.
pub struct SectionLayout {
    pub headers: Vec<ImageSectionHeader>,
    pub offsets: Vec<(usize, usize)>,
    pub max_virtual_end: u32,
    pub total_file_size: usize,
}

/// Input data for building a PE section.
pub struct SectionData<'a> {
    pub linux: &'a [u8],
    pub initrd: &'a [u8],
    pub cmdline: &'a [u8],
    pub dtb: Option<&'a [u8]>,
    pub luks: Option<&'a [u8]>,
}

/// Builds a list of section names and data from the provided `SectionData`.
pub(crate) fn build_section_list<'a>(data: &SectionData<'a>) -> Vec<(&'static str, &'a [u8])> {
    let mut sections = vec![(".cmdline", data.cmdline)];

    if let Some(dtb) = data.dtb {
        sections.push((".dtb", dtb));
    }

    if let Some(luks) = data.luks {
        sections.push((".luks", luks));
    }

    sections.push((".linux", data.linux));
    sections.push((".initrd", data.initrd));

    sections
}

/// Builds PE section headers and computes file offsets for the given sections based on the provided metadata.
pub fn build_headers(metadata: &PeMetadata, sections: &[(&str, &[u8])]) -> SectionLayout {
    let mut headers = Vec::new();
    let mut offsets = Vec::new();
    let mut current_file_offset = align_to(metadata.last_section_file_end, metadata.file_alignment);
    let mut current_virtual_address = align_to(
        metadata.last_section_virtual_end,
        metadata.section_alignment,
    );
    let mut max_virtual_end = metadata.last_section_virtual_end;

    for (name, data) in sections {
        let virtual_size = data.len() as u32;
        let size_of_raw_data = align_to(virtual_size, metadata.file_alignment);

        let mut section = ImageSectionHeader::default();

        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len().min(constants::SECTION_NAME_MAX_LEN);
        section.name[..name_len].copy_from_slice(&name_bytes[..name_len]);

        section.virtual_size.set(LE, virtual_size);
        section.virtual_address.set(LE, current_virtual_address);
        section.size_of_raw_data.set(LE, size_of_raw_data);
        section.pointer_to_raw_data.set(LE, current_file_offset);

        let characteristics = if *name == ".linux" {
            constants::IMAGE_SCN_CNT_CODE
                | constants::IMAGE_SCN_MEM_EXECUTE
                | constants::IMAGE_SCN_MEM_READ
        } else {
            constants::IMAGE_SCN_CNT_INITIALIZED_DATA | constants::IMAGE_SCN_MEM_READ
        };
        section.characteristics.set(LE, characteristics);

        max_virtual_end = max_virtual_end
            .max(current_virtual_address + align_to(virtual_size, metadata.section_alignment));

        headers.push(section);
        offsets.push((current_file_offset as usize, data.len()));
        current_file_offset += size_of_raw_data;
        current_virtual_address += align_to(virtual_size, metadata.section_alignment);
    }

    SectionLayout {
        headers,
        offsets,
        max_virtual_end,
        total_file_size: current_file_offset as usize,
    }
}

/// Writes the provided section headers and data into the given PE image buffer at the appropriate offsets.
pub fn write_to_image(
    stub: &mut [u8],
    metadata: &PeMetadata,
    layout: &SectionLayout,
    sections: &[(&str, &[u8])],
) -> Result<(), YukiError> {
    for (i, section_header) in layout.headers.iter().enumerate() {
        let offset = metadata.section_table_offset
            + (metadata.current_section_count as usize + i) * mem::size_of::<ImageSectionHeader>();
        let header_bytes = section_header_to_bytes(section_header);
        let end = offset + header_bytes.len();
        if end > stub.len() {
            return Err(YukiError::InvalidPeStructure(format!(
                "Section header offset out of bounds: {offset}-{end}"
            )));
        }
        stub[offset..end].copy_from_slice(&header_bytes);
    }

    for (i, (file_offset, data_len)) in layout.offsets.iter().enumerate() {
        let end = file_offset + data_len;
        if end > stub.len() {
            return Err(YukiError::InvalidPeStructure(format!(
                "Section data offset out of bounds: {file_offset}-{end}"
            )));
        }
        let data = sections[i].1;
        stub[*file_offset..end].copy_from_slice(data);
    }

    Ok(())
}

/// Converts an `ImageSectionHeader` into its raw byte representation for writing into the PE image.
fn section_header_to_bytes(
    header: &ImageSectionHeader,
) -> [u8; mem::size_of::<ImageSectionHeader>()] {
    let mut bytes = [0_u8; mem::size_of::<ImageSectionHeader>()];

    bytes[0..8].copy_from_slice(&header.name);
    bytes[8..12].copy_from_slice(&header.virtual_size.get(LE).to_le_bytes());
    bytes[12..16].copy_from_slice(&header.virtual_address.get(LE).to_le_bytes());
    bytes[16..20].copy_from_slice(&header.size_of_raw_data.get(LE).to_le_bytes());
    bytes[20..24].copy_from_slice(&header.pointer_to_raw_data.get(LE).to_le_bytes());
    bytes[24..36].copy_from_slice(&[0_u8; 12]);
    bytes[36..40].copy_from_slice(&header.characteristics.get(LE).to_le_bytes());

    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_metadata() -> PeMetadata {
        PeMetadata {
            file_header_offset: 0,
            optional_header_offset: 0,
            section_table_offset: 0,
            section_alignment: 4096,
            file_alignment: 512,
            last_section_file_end: 512,
            last_section_virtual_end: 4096,
            current_section_count: 1,
        }
    }

    #[test]
    fn section_header_to_bytes_basic() {
        // ARRANGE
        let mut header = ImageSectionHeader::default();
        header.name[0..5].copy_from_slice(b".text");
        header.virtual_size.set(LE, 1000u32);
        header.virtual_address.set(LE, 4096u32);
        header.size_of_raw_data.set(LE, 512u32);
        header.pointer_to_raw_data.set(LE, 512u32);
        header.characteristics.set(LE, 0x60000020u32);

        // ACT
        let bytes = section_header_to_bytes(&header);

        // ASSERT
        assert_eq!(bytes[0..5], *b".text");
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 1000);
        assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 4096);
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 512);
        assert_eq!(u32::from_le_bytes(bytes[20..24].try_into().unwrap()), 512);
        assert_eq!(
            u32::from_le_bytes(bytes[36..40].try_into().unwrap()),
            0x60000020
        );
    }

    #[test]
    fn section_header_to_bytes_all_zeros() {
        // ARRANGE
        let header = ImageSectionHeader::default();

        // ACT
        let bytes = section_header_to_bytes(&header);

        // ASSERT
        for byte in &bytes[24..36] {
            assert_eq!(*byte, 0);
        }
    }

    #[test]
    fn section_header_to_bytes_pads_correctly() {
        // ARRANGE
        let header = ImageSectionHeader::default();

        // ACT
        let bytes = section_header_to_bytes(&header);

        // ASSERT
        assert_eq!(bytes.len(), mem::size_of::<ImageSectionHeader>());
        assert_eq!(bytes[24..36], [0u8; 12]);
    }

    #[test]
    fn section_header_to_bytes_big_values() {
        // ARRANGE
        let mut header = ImageSectionHeader::default();
        header.virtual_size.set(LE, u32::MAX);
        header.virtual_address.set(LE, u32::MAX - 1);
        header.size_of_raw_data.set(LE, u32::MAX - 2);
        header.pointer_to_raw_data.set(LE, u32::MAX - 3);

        // ACT
        let bytes = section_header_to_bytes(&header);

        // ASSERT
        assert_eq!(
            u32::from_le_bytes(bytes[12..16].try_into().unwrap()),
            u32::MAX - 1
        );
        assert_eq!(
            u32::from_le_bytes(bytes[16..20].try_into().unwrap()),
            u32::MAX - 2
        );
        assert_eq!(
            u32::from_le_bytes(bytes[20..24].try_into().unwrap()),
            u32::MAX - 3
        );
    }

    #[test]
    fn build_headers_success() {
        // ARRANGE
        let metadata = create_test_metadata();
        let linux = vec![0u8; 1024];
        let initrd = vec![0u8; 2048];
        let cmdline = b"console=ttyS0";
        let data = SectionData {
            linux: &linux,
            initrd: &initrd,
            cmdline,
            dtb: None,
            luks: None,
        };
        let sections = build_section_list(&data);

        // ACT
        let layout = build_headers(&metadata, &sections);

        // ASSERT
        assert_eq!(layout.headers.len(), 3);
        assert_eq!(layout.offsets.len(), 3);
        assert_eq!(layout.headers[0].name[0..8], *b".cmdline");
        assert_eq!(layout.headers[1].name[0..6], *b".linux");
        assert_eq!(layout.headers[2].name[0..7], *b".initrd");
    }

    #[test]
    fn build_headers_offsets_alignment() {
        // ARRANGE
        let metadata = create_test_metadata();
        let linux = vec![0u8; 100];
        let initrd = vec![0u8; 200];
        let cmdline = b"test";
        let data = SectionData {
            linux: &linux,
            initrd: &initrd,
            cmdline,
            dtb: None,
            luks: None,
        };
        let sections = build_section_list(&data);

        // ACT
        let layout = build_headers(&metadata, &sections);

        // ASSERT
        let first_offset = layout.offsets[0].0;
        assert_eq!(first_offset % metadata.file_alignment as usize, 0);
        for (offset, len) in layout.offsets.iter().skip(1) {
            assert!(*offset > 0, "section offset should be positive");
            assert!(*len > 0, "section data length should be positive");
        }
    }

    #[test]
    fn build_headers_characteristics() {
        // ARRANGE
        let metadata = create_test_metadata();
        let linux = vec![0u8; 100];
        let initrd = vec![0u8; 100];
        let cmdline = b"test";
        let data = SectionData {
            linux: &linux,
            initrd: &initrd,
            cmdline,
            dtb: None,
            luks: None,
        };
        let sections = build_section_list(&data);

        // ACT
        let layout = build_headers(&metadata, &sections);

        // ASSERT
        let linux_chars = layout.headers[1].characteristics.get(LE);
        let cmdline_chars = layout.headers[0].characteristics.get(LE);
        assert_eq!(
            linux_chars,
            constants::IMAGE_SCN_CNT_CODE
                | constants::IMAGE_SCN_MEM_EXECUTE
                | constants::IMAGE_SCN_MEM_READ
        );
        assert_eq!(
            cmdline_chars,
            constants::IMAGE_SCN_CNT_INITIALIZED_DATA | constants::IMAGE_SCN_MEM_READ
        );
    }

    #[test]
    fn build_headers_max_virtual_end() {
        // ARRANGE
        let metadata = create_test_metadata();
        let linux = vec![0u8; 1000];
        let initrd = vec![0u8; 1000];
        let cmdline = b"test";
        let data = SectionData {
            linux: &linux,
            initrd: &initrd,
            cmdline,
            dtb: None,
            luks: None,
        };
        let sections = build_section_list(&data);

        // ACT
        let layout = build_headers(&metadata, &sections);

        // ASSERT
        assert!(layout.max_virtual_end > metadata.last_section_virtual_end);
    }

    #[test]
    fn build_headers_name_truncation() {
        // ARRANGE
        let metadata = create_test_metadata();
        let linux = vec![0u8; 100];
        let initrd = vec![0u8; 100];
        let cmdline = b"very_long_cmdline_name";
        let data = SectionData {
            linux: &linux,
            initrd: &initrd,
            cmdline,
            dtb: None,
            luks: None,
        };
        let sections = build_section_list(&data);

        // ACT
        let layout = build_headers(&metadata, &sections);

        // ASSERT
        let cmdline_header = &layout.headers[0];
        assert!(cmdline_header.name[0..8].iter().any(|&b| b != 0));
        assert_eq!(cmdline_header.name.len(), 8);
    }

    #[test]
    fn build_headers_sequential_offsets() {
        // ARRANGE
        let metadata = create_test_metadata();
        let linux = vec![0u8; 512];
        let initrd = vec![0u8; 512];
        let cmdline = b"cmd";
        let data = SectionData {
            linux: &linux,
            initrd: &initrd,
            cmdline,
            dtb: None,
            luks: None,
        };
        let sections = build_section_list(&data);

        // ACT
        let layout = build_headers(&metadata, &sections);

        // ASSERT
        for i in 1..layout.offsets.len() {
            let (prev_offset, prev_len) = layout.offsets[i - 1];
            let (curr_offset, _) = layout.offsets[i];
            assert!(curr_offset >= prev_offset + prev_len);
        }
    }

    #[test]
    fn build_headers_with_dtb() {
        // ARRANGE
        let metadata = create_test_metadata();
        let linux = vec![0u8; 1024];
        let initrd = vec![0u8; 2048];
        let cmdline = b"console=ttyS0";
        let dtb = vec![0xd0, 0x0d, 0xfe, 0xed];
        let data = SectionData {
            linux: &linux,
            initrd: &initrd,
            cmdline,
            dtb: Some(&dtb),
            luks: None,
        };
        let sections = build_section_list(&data);

        // ACT
        let layout = build_headers(&metadata, &sections);

        // ASSERT
        assert_eq!(layout.headers.len(), 4);
        assert_eq!(layout.offsets.len(), 4);
        assert_eq!(layout.headers[0].name[0..8], *b".cmdline");
        assert_eq!(layout.headers[1].name[0..4], *b".dtb");
        assert_eq!(layout.headers[2].name[0..6], *b".linux");
        assert_eq!(layout.headers[3].name[0..7], *b".initrd");
    }

    #[test]
    fn write_to_image_success() {
        // ARRANGE
        let metadata = create_test_metadata();
        let linux = vec![1u8; 1024];
        let initrd = vec![2u8; 2048];
        let cmdline = b"console=ttyS0";
        let data = SectionData {
            linux: &linux,
            initrd: &initrd,
            cmdline,
            dtb: None,
            luks: None,
        };
        let sections = build_section_list(&data);
        let layout = build_headers(&metadata, &sections);
        let mut stub_data = vec![0u8; 100 * 1024];

        // ACT
        let result = write_to_image(&mut stub_data, &metadata, &layout, &sections);

        // ASSERT
        assert!(result.is_ok(), "write_to_image should succeed");
        for (i, section_header) in layout.headers.iter().enumerate() {
            let offset = metadata.section_table_offset
                + (metadata.current_section_count as usize + i)
                    * mem::size_of::<ImageSectionHeader>();
            let expected_bytes = section_header_to_bytes(section_header);
            assert_eq!(
                &stub_data[offset..offset + expected_bytes.len()],
                &expected_bytes,
                "Section header {} should be written correctly",
                i
            );
        }
        let expected: [&[u8]; 3] = [cmdline, &linux, &initrd];
        for (i, (file_offset, data_len)) in layout.offsets.iter().enumerate() {
            let end = file_offset + data_len;
            assert_eq!(
                &stub_data[*file_offset..end],
                expected[i],
                "Section data {i} mismatch"
            );
        }
    }

    #[test]
    fn write_to_image_buffer_too_small_for_headers() {
        // ARRANGE
        let metadata = create_test_metadata();
        let linux = vec![1u8; 100];
        let initrd = vec![2u8; 100];
        let cmdline = b"test";
        let data = SectionData {
            linux: &linux,
            initrd: &initrd,
            cmdline,
            dtb: None,
            luks: None,
        };
        let sections = build_section_list(&data);
        let layout = build_headers(&metadata, &sections);
        let header_offset = metadata.section_table_offset
            + (metadata.current_section_count as usize) * mem::size_of::<ImageSectionHeader>();
        let mut stub_data = vec![0u8; header_offset + 10];

        // ACT
        let result = write_to_image(&mut stub_data, &metadata, &layout, &sections);

        // ASSERT
        let err = result.expect_err("write_to_image should fail with buffer too small");
        assert_eq!(
            std::mem::discriminant(&err),
            std::mem::discriminant(&YukiError::InvalidPeStructure(String::new())),
            "expected InvalidPeStructure, got: {err:?}"
        );
    }

    #[test]
    fn write_to_image_buffer_too_small_for_data() {
        // ARRANGE
        let metadata = create_test_metadata();
        let linux = vec![1u8; 100];
        let initrd = vec![2u8; 100];
        let cmdline = b"test";
        let data = SectionData {
            linux: &linux,
            initrd: &initrd,
            cmdline,
            dtb: None,
            luks: None,
        };
        let sections = build_section_list(&data);
        let layout = build_headers(&metadata, &sections);
        let data_offset = layout.offsets[0].0;
        let mut stub_data = vec![0u8; data_offset + 10];

        // ACT
        let result = write_to_image(&mut stub_data, &metadata, &layout, &sections);

        // ASSERT
        let err = result.expect_err("write_to_image should fail with buffer too small for data");
        assert_eq!(
            std::mem::discriminant(&err),
            std::mem::discriminant(&YukiError::InvalidPeStructure(String::new())),
            "expected InvalidPeStructure, got: {err:?}"
        );
    }

    #[test]
    fn write_to_image_empty_sections() {
        // ARRANGE
        let metadata = create_test_metadata();
        let linux = vec![];
        let initrd = vec![];
        let cmdline = b"";
        let data = SectionData {
            linux: &linux,
            initrd: &initrd,
            cmdline,
            dtb: None,
            luks: None,
        };
        let sections = build_section_list(&data);
        let layout = build_headers(&metadata, &sections);
        let mut stub_data = vec![0u8; 10 * 1024];

        // ACT
        let result = write_to_image(&mut stub_data, &metadata, &layout, &sections);

        // ASSERT
        assert!(
            result.is_ok(),
            "write_to_image should succeed with empty sections"
        );
        for (file_offset, data_len) in layout.offsets.iter() {
            let end = file_offset + data_len;
            assert_eq!(
                end, *file_offset,
                "Data length should be 0 for empty sections"
            );
        }
    }
}
