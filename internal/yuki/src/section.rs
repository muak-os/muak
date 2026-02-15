//! PE section creation and embedding.

use std::mem;

use object::LittleEndian as LE;
use object::pe::ImageSectionHeader;

use crate::YukiError;
use crate::binary::align_to;
use crate::constants;
use crate::pe::PeMetadata;

pub struct SectionInfo {
    pub headers: Vec<ImageSectionHeader>,
    pub offsets: Vec<(usize, usize)>,
    pub max_virtual_end: u32,
    pub total_file_size: usize,
}

pub struct SectionData<'a> {
    pub linux: &'a [u8],
    pub initrd: &'a [u8],
    pub cmdline: &'a [u8],
    pub dtb: Option<&'a [u8]>,
    pub luks: Option<&'a [u8]>,
}

fn build_section_list<'a>(data: &SectionData<'a>) -> Vec<(&'static str, &'a [u8])> {
    let mut sections = vec![(".linux", data.linux)];

    if let Some(dtb) = data.dtb {
        sections.push((".dtb", dtb));
    }

    if let Some(luks) = data.luks {
        sections.push((".luks", luks));
    }

    sections.push((".cmdline", data.cmdline));
    sections.push((".initrd", data.initrd));

    sections
}

pub fn build_headers(metadata: &PeMetadata, data: &SectionData) -> Result<SectionInfo, YukiError> {
    let sections_to_add = build_section_list(data);

    let mut headers = Vec::new();
    let mut offsets = Vec::new();
    let mut current_file_offset = align_to(metadata.last_section_file_end, metadata.file_alignment);
    let mut current_virtual_address = align_to(
        metadata.last_section_virtual_end,
        metadata.section_alignment,
    );
    let mut max_virtual_end = metadata.last_section_virtual_end;

    for (name, data) in &sections_to_add {
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

    Ok(SectionInfo {
        headers,
        offsets,
        max_virtual_end,
        total_file_size: current_file_offset as usize,
    })
}

pub fn write_to_image(
    stub_data: &mut [u8],
    metadata: &PeMetadata,
    section_info: &SectionInfo,
    data: &SectionData,
) -> Result<(), YukiError> {
    let sections_to_add = build_section_list(data);

    for (i, section_header) in section_info.headers.iter().enumerate() {
        let offset = metadata.section_table_offset
            + (metadata.current_section_count as usize + i) * mem::size_of::<ImageSectionHeader>();
        let header_bytes = section_header_to_bytes(section_header);
        let end = offset
            .checked_add(header_bytes.len())
            .ok_or(YukiError::InvalidPeStructure(
                "Section header offset overflow".to_string(),
            ))?;
        if end > stub_data.len() {
            return Err(YukiError::InvalidPeStructure(format!(
                "Section header offset out of bounds: {}-{}",
                offset, end
            )));
        }
        stub_data[offset..end].copy_from_slice(&header_bytes);
    }

    for (i, (file_offset, data_len)) in section_info.offsets.iter().enumerate() {
        let end = file_offset
            .checked_add(*data_len)
            .ok_or(YukiError::InvalidPeStructure(
                "Section data offset overflow".to_string(),
            ))?;
        if end > stub_data.len() {
            return Err(YukiError::InvalidPeStructure(format!(
                "Section data offset out of bounds: {}-{}",
                file_offset, end
            )));
        }
        let data = sections_to_add[i].1;
        stub_data[*file_offset..end].copy_from_slice(data);
    }

    Ok(())
}

fn section_header_to_bytes(
    header: &ImageSectionHeader,
) -> [u8; mem::size_of::<ImageSectionHeader>()] {
    let mut bytes = [0u8; mem::size_of::<ImageSectionHeader>()];

    bytes[0..8].copy_from_slice(&header.name);
    bytes[8..12].copy_from_slice(&header.virtual_size.get(LE).to_le_bytes());
    bytes[12..16].copy_from_slice(&header.virtual_address.get(LE).to_le_bytes());
    bytes[16..20].copy_from_slice(&header.size_of_raw_data.get(LE).to_le_bytes());
    bytes[20..24].copy_from_slice(&header.pointer_to_raw_data.get(LE).to_le_bytes());
    bytes[24..36].copy_from_slice(&[0u8; 12]);
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
    fn test_section_header_to_bytes_basic() {
        let mut header = ImageSectionHeader::default();
        header.name[0..5].copy_from_slice(b".text");
        header.virtual_size.set(LE, 1000u32);
        header.virtual_address.set(LE, 4096u32);
        header.size_of_raw_data.set(LE, 512u32);
        header.pointer_to_raw_data.set(LE, 512u32);
        header.characteristics.set(LE, 0x60000020u32);

        let bytes = section_header_to_bytes(&header);

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
    fn test_section_header_to_bytes_all_zeros() {
        let header = ImageSectionHeader::default();
        let bytes = section_header_to_bytes(&header);

        for byte in &bytes[24..36] {
            assert_eq!(*byte, 0);
        }
    }

    #[test]
    fn test_section_header_to_bytes_pads_correctly() {
        let header = ImageSectionHeader::default();
        let bytes = section_header_to_bytes(&header);

        assert_eq!(bytes.len(), mem::size_of::<ImageSectionHeader>());
        assert_eq!(bytes[24..36], [0u8; 12]);
    }

    #[test]
    fn test_section_header_to_bytes_big_values() {
        let mut header = ImageSectionHeader::default();
        header.virtual_size.set(LE, u32::MAX);
        header.virtual_address.set(LE, u32::MAX - 1);
        header.size_of_raw_data.set(LE, u32::MAX - 2);
        header.pointer_to_raw_data.set(LE, u32::MAX - 3);

        let bytes = section_header_to_bytes(&header);

        assert_eq!(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            u32::MAX
        );
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
    fn test_build_headers_success() {
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

        let section_info = build_headers(&metadata, &data).expect("Should build");

        assert_eq!(section_info.headers.len(), 3);
        assert_eq!(section_info.offsets.len(), 3);
        assert_eq!(section_info.headers[0].name[0..8], *b".cmdline");
        assert_eq!(section_info.headers[1].name[0..6], *b".linux");
        assert_eq!(section_info.headers[2].name[0..7], *b".initrd");
    }

    #[test]
    fn test_build_headers_offsets_alignment() {
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

        let section_info = build_headers(&metadata, &data).expect("Should build");

        let first_offset = section_info.offsets[0].0;
        assert_eq!(first_offset % metadata.file_alignment as usize, 0);

        for i in 0..section_info.offsets.len() {
            let (offset, len) = section_info.offsets[i];
            assert!(offset > 0 || i == 0);
            assert!(len > 0 || i == 0); // cmdline can be empty
        }
    }

    #[test]
    fn test_build_headers_characteristics() {
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

        let section_info = build_headers(&metadata, &data).expect("Should build");

        let linux_header = &section_info.headers[1];
        let cmdline_header = &section_info.headers[0];

        let linux_chars = linux_header.characteristics.get(LE);
        let cmdline_chars = cmdline_header.characteristics.get(LE);

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
    fn test_build_headers_max_virtual_end() {
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

        let section_info = build_headers(&metadata, &data).expect("Should build");

        assert!(section_info.max_virtual_end > metadata.last_section_virtual_end);
    }

    #[test]
    fn test_build_headers_name_truncation() {
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

        let section_info = build_headers(&metadata, &data).expect("Should build");

        let cmdline_header = &section_info.headers[0];
        assert!(cmdline_header.name[0..8].iter().any(|&b| b != 0));
        assert_eq!(cmdline_header.name.len(), 8);
    }

    #[test]
    fn test_build_headers_sequential_offsets() {
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

        let section_info = build_headers(&metadata, &data).expect("Should build");

        for i in 1..section_info.offsets.len() {
            let (prev_offset, prev_len) = section_info.offsets[i - 1];
            let (curr_offset, _) = section_info.offsets[i];
            assert!(curr_offset >= prev_offset + prev_len);
        }
    }

    #[test]
    fn test_build_headers_with_dtb() {
        let metadata = create_test_metadata();
        let linux = vec![0u8; 1024];
        let initrd = vec![0u8; 2048];
        let cmdline = b"console=ttyS0";
        let dtb = vec![0xd0, 0x0d, 0xfe, 0xed]; // DTB magic header

        let data = SectionData {
            linux: &linux,
            initrd: &initrd,
            cmdline,
            dtb: Some(&dtb),
            luks: None,
        };

        let section_info = build_headers(&metadata, &data).expect("Should build");

        assert_eq!(section_info.headers.len(), 4);
        assert_eq!(section_info.offsets.len(), 4);
        assert_eq!(section_info.headers[0].name[0..8], *b".cmdline");
        assert_eq!(section_info.headers[1].name[0..4], *b".dtb");
        assert_eq!(section_info.headers[2].name[0..6], *b".linux");
        assert_eq!(section_info.headers[3].name[0..7], *b".initrd");
    }

    #[test]
    fn test_write_to_image_success() {
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

        let section_info = build_headers(&metadata, &data).expect("Should build");

        let mut stub_data = vec![0u8; 100 * 1024]; // 100KB should be plenty

        let result = write_to_image(&mut stub_data, &metadata, &section_info, &data);

        assert!(result.is_ok(), "write_to_image should succeed");

        // Verify section headers
        for (i, section_header) in section_info.headers.iter().enumerate() {
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

        // Verify data
        for (i, (file_offset, data_len)) in section_info.offsets.iter().enumerate() {
            let end = file_offset + data_len;
            match i {
                0 => assert_eq!(
                    &stub_data[*file_offset..end],
                    cmdline,
                    "Cmdline data should be copied"
                ),
                1 => assert_eq!(
                    &stub_data[*file_offset..end],
                    &linux,
                    "Linux data should be copied"
                ),
                2 => assert_eq!(
                    &stub_data[*file_offset..end],
                    &initrd,
                    "Initrd data should be copied"
                ),
                _ => panic!("Unexpected section index"),
            }
        }
    }

    #[test]
    fn test_write_to_image_buffer_too_small_for_headers() {
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

        let section_info = build_headers(&metadata, &data).expect("Should build");

        let header_offset = metadata.section_table_offset
            + (metadata.current_section_count as usize) * mem::size_of::<ImageSectionHeader>();
        let mut stub_data = vec![0u8; header_offset + 10]; // Too small

        let result = write_to_image(&mut stub_data, &metadata, &section_info, &data);

        assert!(
            result.is_err(),
            "write_to_image should fail with buffer too small"
        );
        assert!(matches!(
            result.unwrap_err(),
            YukiError::InvalidPeStructure(_)
        ));
    }

    #[test]
    fn test_write_to_image_buffer_too_small_for_data() {
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

        let section_info = build_headers(&metadata, &data).expect("Should build");

        let data_offset = section_info.offsets[0].0;
        let mut stub_data = vec![0u8; data_offset + 10]; // Too small for first data

        let result = write_to_image(&mut stub_data, &metadata, &section_info, &data);

        assert!(
            result.is_err(),
            "write_to_image should fail with buffer too small for data"
        );
        assert!(matches!(
            result.unwrap_err(),
            YukiError::InvalidPeStructure(_)
        ));
    }

    #[test]
    fn test_write_to_image_empty_sections() {
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

        let section_info = build_headers(&metadata, &data).expect("Should build");

        let mut stub_data = vec![0u8; 10 * 1024];

        let result = write_to_image(&mut stub_data, &metadata, &section_info, &data);

        assert!(
            result.is_ok(),
            "write_to_image should succeed with empty sections"
        );

        for (file_offset, data_len) in section_info.offsets.iter() {
            let end = file_offset + data_len;
            assert_eq!(
                end, *file_offset,
                "Data length should be 0 for empty sections"
            );
        }
    }
}
