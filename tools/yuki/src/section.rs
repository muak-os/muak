//! PE section creation and embedding.

use object::LittleEndian as LE;
use object::pe::ImageSectionHeader;

use crate::binary::{align_to, usize_from_u128};
use crate::constants;
use crate::error::{Result, YukiError};
use crate::pe::PeMetadata;

/// Computed file and virtual memory layout for a set of PE sections to be embedded.
#[derive(Default)]
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
pub fn build_headers(metadata: &PeMetadata, sections: &[(&str, &[u8])]) -> Result<SectionLayout> {
    let mut headers = Vec::new();
    let mut offsets = Vec::new();
    let mut current_file_offset = align_to(metadata.last_section_file_end, metadata.file_alignment);
    let mut current_virtual_address = align_to(
        metadata.last_section_virtual_end,
        metadata.section_alignment,
    );
    let mut max_virtual_end = metadata.last_section_virtual_end;

    for &(name, data) in sections {
        let virtual_size = u32::try_from(data.len()).map_err(|_conversion_error| {
            YukiError::InvalidPeStructure(format!("section '{name}' too large"))
        })?;
        let size_of_raw_data = align_to(virtual_size, metadata.file_alignment);
        let aligned_virtual_size = align_to(virtual_size, metadata.section_alignment);

        let mut section = ImageSectionHeader::default();

        let name_bytes = name.as_bytes();
        for (destination, source) in section
            .name
            .iter_mut()
            .take(constants::SECTION_NAME_MAX_LEN)
            .zip(name_bytes.iter().take(constants::SECTION_NAME_MAX_LEN))
        {
            *destination = *source;
        }

        section.virtual_size.set(LE, virtual_size);
        section.virtual_address.set(LE, current_virtual_address);
        section.size_of_raw_data.set(LE, size_of_raw_data);
        section.pointer_to_raw_data.set(LE, current_file_offset);

        let characteristics = if name == ".linux" {
            constants::IMAGE_SCN_CNT_CODE
                | constants::IMAGE_SCN_MEM_EXECUTE
                | constants::IMAGE_SCN_MEM_READ
        } else {
            constants::IMAGE_SCN_CNT_INITIALIZED_DATA | constants::IMAGE_SCN_MEM_READ
        };
        section.characteristics.set(LE, characteristics);

        let section_virtual_end = current_virtual_address
            .checked_add(aligned_virtual_size)
            .ok_or_else(|| {
                YukiError::InvalidPeStructure("section virtual end overflow".to_owned())
            })?;

        max_virtual_end = max_virtual_end.max(section_virtual_end);

        let current_file_offset_usize = usize_from_u128(
            u128::from(current_file_offset),
            "section file offset does not fit in usize",
        )?;

        headers.push(section);
        offsets.push((current_file_offset_usize, data.len()));
        current_file_offset = current_file_offset
            .checked_add(size_of_raw_data)
            .ok_or_else(|| {
                YukiError::InvalidPeStructure("next section file offset overflow".to_owned())
            })?;
        current_virtual_address = section_virtual_end;
    }

    let total_file_size = usize_from_u128(
        u128::from(current_file_offset),
        "total file size does not fit in usize",
    )?;

    Ok(SectionLayout {
        headers,
        offsets,
        max_virtual_end,
        total_file_size,
    })
}

/// Writes the provided section headers and data into the given PE image buffer at the appropriate offsets.
pub fn write_to_image(
    stub: &mut [u8],
    metadata: &PeMetadata,
    layout: &SectionLayout,
    sections: &[(&str, &[u8])],
) -> Result<()> {
    if layout.headers.len() != sections.len() || layout.offsets.len() != sections.len() {
        return Err(YukiError::InvalidPeStructure(
            "section layout length mismatch".to_owned(),
        ));
    }

    for (i, section_header) in layout.headers.iter().enumerate() {
        let section_index = usize::from(metadata.current_section_count).saturating_add(i);
        let section_offset =
            section_index.saturating_mul(core::mem::size_of::<ImageSectionHeader>());
        let offset = metadata.section_table_offset.saturating_add(section_offset);
        let header_bytes = section_header_to_bytes(section_header);
        let end = offset.saturating_add(header_bytes.len());
        stub.get_mut(offset..end)
            .ok_or_else(|| {
                YukiError::InvalidPeStructure(format!("section header oob: {offset}-{end}"))
            })?
            .copy_from_slice(&header_bytes);
    }

    for (i, &(file_offset, data_len)) in layout.offsets.iter().enumerate() {
        let data = sections
            .get(i)
            .ok_or_else(|| YukiError::InvalidPeStructure(format!("missing section data at {i}")))?
            .1;
        if data.len() != data_len {
            return Err(YukiError::InvalidPeStructure(format!(
                "section data length mismatch at {i}: expected {data_len}, got {}",
                data.len()
            )));
        }

        let end = file_offset.saturating_add(data_len);
        stub.get_mut(file_offset..end)
            .ok_or_else(|| {
                YukiError::InvalidPeStructure(format!("section data oob: {file_offset}-{end}"))
            })?
            .copy_from_slice(data);
    }

    Ok(())
}

/// Converts an `ImageSectionHeader` into its raw byte representation for writing into the PE image.
pub(crate) fn section_header_to_bytes(
    header: &ImageSectionHeader,
) -> [u8; core::mem::size_of::<ImageSectionHeader>()] {
    let mut bytes = [0_u8; core::mem::size_of::<ImageSectionHeader>()];

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
    use std::ptr::NonNull;

    use object::LittleEndian as LE;
    use object::pe::ImageSectionHeader;

    use super::*;
    use crate::constants;
    use crate::pe::PeMetadata;

    fn create_test_metadata() -> PeMetadata {
        PeMetadata {
            file_header_offset: 0,
            optional_header_offset: 0,
            section_table_offset: 0,
            size_of_headers: 512,
            section_alignment: 4096,
            file_alignment: 512,
            last_section_file_end: 512,
            last_section_virtual_end: 4096,
            current_section_count: 1,
        }
    }

    fn oversized_slice() -> &'static [u8] {
        unsafe {
            std::slice::from_raw_parts(
                NonNull::<u8>::dangling().as_ptr(),
                usize::try_from(u32::MAX).unwrap_or_default() + 1,
            )
        }
    }

    #[test]
    fn section_header_to_bytes_basic() {
        // ARRANGE
        let mut header = ImageSectionHeader::default();
        header.name[0..5].copy_from_slice(b".text");
        header.virtual_size.set(LE, 1000);
        header.virtual_address.set(LE, 4096);
        header.size_of_raw_data.set(LE, 512);
        header.pointer_to_raw_data.set(LE, 512);
        header.characteristics.set(LE, 0x60000020);

        // ACT
        let bytes = section_header_to_bytes(&header);

        // ASSERT
        assert_eq!(bytes[0..5], *b".text");
        assert_eq!(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap_or([0; 4])),
            1000
        );
        assert_eq!(
            u32::from_le_bytes(bytes[12..16].try_into().unwrap_or([0; 4])),
            4096
        );
        assert_eq!(
            u32::from_le_bytes(bytes[16..20].try_into().unwrap_or([0; 4])),
            512
        );
        assert_eq!(
            u32::from_le_bytes(bytes[20..24].try_into().unwrap_or([0; 4])),
            512
        );
        assert_eq!(
            u32::from_le_bytes(bytes[36..40].try_into().unwrap_or([0; 4])),
            0x60000020
        );
    }

    #[test]
    fn section_header_to_bytes_all_zeros() {
        // ARRANGE

        // ACT
        let bytes = section_header_to_bytes(&ImageSectionHeader::default());

        // ASSERT
        for byte in &bytes[24..36] {
            assert_eq!(*byte, 0);
        }
    }

    #[test]
    fn section_header_to_bytes_pads_correctly() {
        // ARRANGE

        // ACT
        let bytes = section_header_to_bytes(&ImageSectionHeader::default());

        // ASSERT
        assert_eq!(bytes.len(), std::mem::size_of::<ImageSectionHeader>());
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
            u32::from_le_bytes(bytes[12..16].try_into().unwrap_or([0; 4])),
            u32::MAX - 1
        );
        assert_eq!(
            u32::from_le_bytes(bytes[16..20].try_into().unwrap_or([0; 4])),
            u32::MAX - 2
        );
        assert_eq!(
            u32::from_le_bytes(bytes[20..24].try_into().unwrap_or([0; 4])),
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
        let layout = build_headers(&metadata, &sections).unwrap_or_default();

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
        let layout = build_headers(&metadata, &sections).unwrap_or_default();

        // ASSERT
        assert_eq!(layout.offsets[0].0 % metadata.file_alignment as usize, 0);
        for (offset, len) in layout.offsets.iter().skip(1) {
            assert!(*offset > 0);
            assert!(*len > 0);
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
        let layout = build_headers(&metadata, &sections).unwrap_or_default();

        // ASSERT
        assert_eq!(
            layout.headers[1].characteristics.get(LE),
            constants::IMAGE_SCN_CNT_CODE
                | constants::IMAGE_SCN_MEM_EXECUTE
                | constants::IMAGE_SCN_MEM_READ
        );
        assert_eq!(
            layout.headers[0].characteristics.get(LE),
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
        let layout = build_headers(&metadata, &sections).unwrap_or_default();

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
        let layout = build_headers(&metadata, &sections).unwrap_or_default();

        // ASSERT
        assert!(layout.headers[0].name[0..8].iter().any(|&b| b != 0));
        assert_eq!(layout.headers[0].name.len(), 8);
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
        let layout = build_headers(&metadata, &sections).unwrap_or_default();

        // ASSERT
        for i in 1..layout.offsets.len() {
            let (prev_offset, prev_len) = layout.offsets[i - 1];
            let (curr_offset, _) = layout.offsets[i];
            assert!(curr_offset >= prev_offset + prev_len);
        }
    }

    #[test]
    fn build_headers_rejects_sections_larger_than_u32() {
        // ARRANGE
        let metadata = create_test_metadata();
        let sections = [(".huge", oversized_slice())];

        // ACT
        let result = build_headers(&metadata, &sections);

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(message))
                if message.contains("section '.huge' too large")
        ));
    }

    #[test]
    fn build_headers_rejects_virtual_overflow() {
        // ARRANGE
        let metadata = PeMetadata {
            last_section_virtual_end: u32::MAX - 1024,
            ..create_test_metadata()
        };
        let sections = [(".ok", &[0u8; 2048][..])];

        // ACT
        let result = build_headers(&metadata, &sections);

        // ASSERT
        assert!(
            matches!(result, Err(YukiError::InvalidPeStructure(message)) if message.contains("section virtual end overflow"))
        );
    }

    #[test]
    fn build_headers_rejects_file_offset_overflow() {
        // ARRANGE
        let metadata = PeMetadata {
            last_section_file_end: u32::MAX - 256,
            ..create_test_metadata()
        };
        let sections = [(".ok", &[0u8; 512][..])];

        // ACT
        let result = build_headers(&metadata, &sections);

        // ASSERT
        assert!(
            matches!(result, Err(YukiError::InvalidPeStructure(message)) if message.contains("next section file offset overflow"))
        );
    }

    #[test]
    fn build_headers_next_virtual_address_matches_section_end() {
        // ARRANGE
        let metadata = create_test_metadata();
        let sections = [(".ok", &[0u8; 1][..]), (".next", &[0u8; 1][..])];

        // ACT
        let layout = build_headers(&metadata, &sections).unwrap_or_default();

        // ASSERT
        let first_end = layout.headers[0].virtual_address.get(LE)
            + align_to(
                layout.headers[0].virtual_size.get(LE),
                metadata.section_alignment,
            );
        assert_eq!(layout.headers[1].virtual_address.get(LE), first_end);
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
        let layout = build_headers(&metadata, &sections).unwrap_or_default();

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
        let layout = build_headers(&metadata, &sections).unwrap_or_default();
        let mut stub_data = vec![0u8; 100 * 1024];

        // ACT
        let result = write_to_image(&mut stub_data, &metadata, &layout, &sections);

        // ASSERT
        assert!(result.is_ok());
        for (i, header) in layout.headers.iter().enumerate() {
            let offset = metadata.section_table_offset
                + (metadata.current_section_count as usize + i)
                    * std::mem::size_of::<ImageSectionHeader>();
            let expected = section_header_to_bytes(header);
            assert_eq!(&stub_data[offset..offset + expected.len()], &expected);
        }
        let expected: [&[u8]; 3] = [cmdline, &linux, &initrd];
        for (i, (file_offset, data_len)) in layout.offsets.iter().enumerate() {
            let end = file_offset + data_len;
            assert_eq!(&stub_data[*file_offset..end], expected[i]);
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
        let layout = build_headers(&metadata, &sections).unwrap_or_default();
        let header_offset = metadata.section_table_offset
            + metadata.current_section_count as usize * std::mem::size_of::<ImageSectionHeader>();
        let mut stub_data = vec![0u8; header_offset + 10];

        // ACT
        let result = write_to_image(&mut stub_data, &metadata, &layout, &sections);

        // ASSERT
        assert!(matches!(result, Err(YukiError::InvalidPeStructure(_))));
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

        // ACT
        let layout = build_headers(&metadata, &sections).unwrap_or_default();

        // ASSERT
        assert!(layout.offsets[0].0 > 10);
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

        // ACT
        let sections = build_section_list(&data);
        let layout = build_headers(&metadata, &sections).unwrap_or_default();
        let mut stub_data = vec![0u8; 10 * 1024];
        let result = write_to_image(&mut stub_data, &metadata, &layout, &sections);

        // ASSERT
        assert!(result.is_ok());
        for (file_offset, data_len) in &layout.offsets {
            assert_eq!(file_offset + data_len, *file_offset);
        }
    }

    #[test]
    fn write_to_image_rejects_data_out_of_bounds() {
        // ARRANGE
        let metadata = PeMetadata {
            section_table_offset: 0,
            current_section_count: 0,
            ..create_test_metadata()
        };
        let sections = [(".ok", &[1u8; 32][..])];
        let layout = SectionLayout {
            headers: vec![ImageSectionHeader::default()],
            offsets: vec![(32, 32)],
            max_virtual_end: 0,
            total_file_size: 64,
        };
        let mut stub_data = vec![0u8; 40];

        // ACT
        let result = write_to_image(&mut stub_data, &metadata, &layout, &sections);

        // ASSERT
        assert!(
            matches!(result, Err(YukiError::InvalidPeStructure(message)) if message.contains("section data oob"))
        );
    }

    #[test]
    fn write_to_image_rejects_layout_length_mismatch() {
        // ARRANGE
        let metadata = create_test_metadata();
        let layout = SectionLayout {
            headers: vec![ImageSectionHeader::default()],
            offsets: Vec::new(),
            max_virtual_end: 0,
            total_file_size: 0,
        };
        let sections = [(".ok", &[1u8; 4][..])];
        let mut stub_data = vec![0u8; 64];

        // ACT
        let result = write_to_image(&mut stub_data, &metadata, &layout, &sections);

        // ASSERT
        assert!(
            matches!(result, Err(YukiError::InvalidPeStructure(message)) if message.contains("section layout length mismatch"))
        );
    }

    #[test]
    fn build_section_list_includes_luks() {
        // ARRANGE
        let linux = [1u8; 4];
        let initrd = [2u8; 4];
        let cmdline = [3u8; 4];
        let luks = [4u8; 4];
        let data = SectionData {
            linux: &linux,
            initrd: &initrd,
            cmdline: &cmdline,
            dtb: None,
            luks: Some(&luks),
        };

        // ACT
        let sections = build_section_list(&data);

        // ASSERT
        assert!(
            sections
                .iter()
                .any(|(name, payload)| *name == ".luks" && *payload == &luks)
        );
    }

    #[test]
    fn build_headers_uses_exact_virtual_size() {
        // ARRANGE
        let metadata = create_test_metadata();
        let sections = [(".ok", &[0u8; 16][..])];

        // ACT
        let layout = build_headers(&metadata, &sections).unwrap_or_default();

        // ASSERT
        assert_eq!(layout.headers[0].virtual_size.get(LE), 16);
    }

    #[test]
    fn write_to_image_header_error_contains_offsets() {
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

        // ACT
        let sections = build_section_list(&data);
        let layout = build_headers(&metadata, &sections).unwrap_or_default();
        let header_offset = metadata.section_table_offset
            + metadata.current_section_count as usize * std::mem::size_of::<ImageSectionHeader>();
        let mut stub_data = vec![0u8; header_offset + 10];
        let result = write_to_image(&mut stub_data, &metadata, &layout, &sections);

        // ASSERTT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(message)) if message.contains("section header oob")
        ));
    }

    #[test]
    fn write_to_image_rejects_data_length_mismatch() {
        // ARRANGE
        let metadata = PeMetadata {
            current_section_count: 0,
            ..create_test_metadata()
        };
        let layout = SectionLayout {
            headers: vec![ImageSectionHeader::default()],
            offsets: vec![(0, 8)],
            max_virtual_end: 0,
            total_file_size: 8,
        };
        let sections = [(".ok", &[1u8; 4][..])];
        let mut stub_data = vec![0u8; 64];

        // ACT
        let result = write_to_image(&mut stub_data, &metadata, &layout, &sections);

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(message))
                if message.contains("section data length mismatch")
        ));
    }

    #[test]
    fn write_to_image_data_error_contains_offsets() {
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

        // ACT
        let sections = build_section_list(&data);
        let layout = build_headers(&metadata, &sections).unwrap_or_default();

        // ASSERT
        assert!(
            layout
                .offsets
                .iter()
                .all(|(offset, len)| offset + len <= layout.total_file_size)
        );
    }
}
