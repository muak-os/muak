//! PE section creation and embedding.

use object::LittleEndian as LE;
use object::pe::ImageSectionHeader;

use crate::binary;
use crate::error::{Result, YukiError};
use crate::pe::PeMetadata;

/// Maximum length of a PE section name in bytes.
const SECTION_NAME_MAX_LEN: usize = 8;

/// PE section characteristic flag: section contains executable code.
const IMAGE_SCN_CNT_CODE: u32 = 0x0000_0020;

/// PE section characteristic flag: section contains initialized data.
const IMAGE_SCN_CNT_INITIALIZED_DATA: u32 = 0x0000_0040;

/// PE section characteristic flag: section is executable in memory.
const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;

/// PE section characteristic flag: section is readable in memory.
const IMAGE_SCN_MEM_READ: u32 = 0x4000_0000;

/// A UKI PE section metadata.
#[derive(Debug, Clone)]
pub struct Section {
    /// PE section name.
    pub name: &'static str,
    /// File offset of the section data within the output PE image.
    pub file_offset: usize,
    /// Size of the section data in bytes.
    pub size: usize,
}

/// Computed file and virtual memory layout for embedded PE sections.
#[derive(Default)]
pub struct Layout {
    /// PE section headers for the new sections.
    pub headers: Vec<ImageSectionHeader>,
    /// Highest virtual address end across all sections.
    pub max_virtual_end: u32,
    /// Total file size of the output PE image.
    pub total_file_size: usize,
}

/// Input data describing the components to embed as PE sections.
pub struct Data<'a> {
    /// Kernel binary data.
    pub linux: &'a [u8],
    /// Initial RAM filesystem data.
    pub initrd: &'a [u8],
    /// Kernel command-line string bytes.
    pub cmdline: &'a [u8],
    /// Optional device-tree blob data.
    pub dtb: Option<&'a [u8]>,
}

/// Builds a list of sections and their data from the provided `Data`.
pub(crate) fn prepare<'a>(data: &Data<'a>) -> (Vec<Section>, Vec<&'a [u8]>) {
    let mut sections = vec![Section {
        name: ".cmdline",
        file_offset: 0,
        size: data.cmdline.len(),
    }];

    let mut section_data: Vec<&[u8]> = vec![data.cmdline];

    if let Some(dtb) = data.dtb {
        sections.push(Section {
            name: ".dtb",
            file_offset: 0,
            size: dtb.len(),
        });
        section_data.push(dtb);
    }

    sections.push(Section {
        name: ".linux",
        file_offset: 0,
        size: data.linux.len(),
    });
    section_data.push(data.linux);

    sections.push(Section {
        name: ".initrd",
        file_offset: 0,
        size: data.initrd.len(),
    });
    section_data.push(data.initrd);

    (sections, section_data)
}

/// Builds PE section headers and computes file offsets for the given sections.
///
/// # Errors
///
/// Returns an error if a section exceeds `u32::MAX` bytes, virtual addresses overflow,
/// or file offsets overflow the PE file size.
pub fn build_headers(metadata: &PeMetadata, sections: &mut [Section]) -> Result<Layout> {
    let mut headers = Vec::new();
    let mut current_file_offset =
        binary::align_to(metadata.last_section_file_end, metadata.file_alignment);
    let mut current_virtual_address = binary::align_to(
        metadata.last_section_virtual_end,
        metadata.section_alignment,
    );
    let mut max_virtual_end = metadata.last_section_virtual_end;

    for section in sections.iter_mut() {
        let virtual_size = u32::try_from(section.size).map_err(|_conversion_error| {
            YukiError::InvalidPeStructure(format!("section '{}' too large", section.name))
        })?;
        let size_of_raw_data = binary::align_to(virtual_size, metadata.file_alignment);
        let aligned_virtual_size = binary::align_to(virtual_size, metadata.section_alignment);

        let mut pe_header = ImageSectionHeader::default();

        let name_bytes = section.name.as_bytes();
        for (destination, source) in pe_header
            .name
            .iter_mut()
            .take(SECTION_NAME_MAX_LEN)
            .zip(name_bytes.iter().take(SECTION_NAME_MAX_LEN))
        {
            *destination = *source;
        }

        pe_header.virtual_size.set(LE, virtual_size);
        pe_header.virtual_address.set(LE, current_virtual_address);
        pe_header.size_of_raw_data.set(LE, size_of_raw_data);
        pe_header.pointer_to_raw_data.set(LE, current_file_offset);

        let characteristics = if section.name == ".linux" {
            IMAGE_SCN_CNT_CODE | IMAGE_SCN_MEM_EXECUTE | IMAGE_SCN_MEM_READ
        } else {
            IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ
        };
        pe_header.characteristics.set(LE, characteristics);

        let section_virtual_end = current_virtual_address
            .checked_add(aligned_virtual_size)
            .ok_or_else(|| {
                YukiError::InvalidPeStructure("section virtual end overflow".to_owned())
            })?;

        max_virtual_end = max_virtual_end.max(section_virtual_end);

        let current_file_offset_usize = binary::usize_from_u128(
            u128::from(current_file_offset),
            "section file offset does not fit in usize",
        )?;

        section.file_offset = current_file_offset_usize;

        headers.push(pe_header);
        current_file_offset = current_file_offset
            .checked_add(size_of_raw_data)
            .ok_or_else(|| {
                YukiError::InvalidPeStructure("next section file offset overflow".to_owned())
            })?;
        current_virtual_address = section_virtual_end;
    }

    let total_file_size = binary::usize_from_u128(
        u128::from(current_file_offset),
        "total file size does not fit in usize",
    )?;

    Ok(Layout {
        headers,
        max_virtual_end,
        total_file_size,
    })
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
    use object::LittleEndian as LE;
    use object::pe::ImageSectionHeader;

    use super::*;
    use crate::pe::PeMetadata;

    fn byte_range(bytes: &[u8], range: core::ops::Range<usize>) -> &[u8] {
        bytes.get(range).unwrap()
    }

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

    fn make_sections(pairs: &[(&'static str, &[u8])]) -> Vec<Section> {
        pairs
            .iter()
            .map(|&(name, data)| Section {
                name,
                file_offset: 0,
                size: data.len(),
            })
            .collect()
    }

    fn hdrs(layout: &Layout, index: usize) -> &ImageSectionHeader {
        layout.headers.get(index).unwrap()
    }

    fn secs(sections: &[Section], index: usize) -> &Section {
        sections.get(index).unwrap()
    }

    #[test]
    fn section_header_to_bytes_basic() {
        // ARRANGE
        let mut header = ImageSectionHeader::default();
        header
            .name
            .get_mut(0..5)
            .unwrap_or_default()
            .copy_from_slice(b".text");
        header.virtual_size.set(LE, 1000);
        header.virtual_address.set(LE, 4096);
        header.size_of_raw_data.set(LE, 512);
        header.pointer_to_raw_data.set(LE, 512);
        header.characteristics.set(LE, 0x6000_0020);

        // ACT
        let bytes = section_header_to_bytes(&header);

        // ASSERT
        assert_eq!(byte_range(&bytes, 0..5), b".text");
        assert_eq!(
            u32::from_le_bytes(byte_range(&bytes, 8..12).try_into().unwrap_or_default()),
            1000
        );
        assert_eq!(
            u32::from_le_bytes(byte_range(&bytes, 12..16).try_into().unwrap_or_default()),
            4096
        );
        assert_eq!(
            u32::from_le_bytes(byte_range(&bytes, 16..20).try_into().unwrap_or_default()),
            512
        );
        assert_eq!(
            u32::from_le_bytes(byte_range(&bytes, 20..24).try_into().unwrap_or_default()),
            512
        );
        assert_eq!(
            u32::from_le_bytes(byte_range(&bytes, 36..40).try_into().unwrap_or_default()),
            0x6000_0020
        );
    }

    #[test]
    fn section_header_to_bytes_all_zeros() {
        // ARRANGE
        // ACT
        let bytes = section_header_to_bytes(&ImageSectionHeader::default());
        // ASSERT
        for byte in byte_range(&bytes, 24..36) {
            assert_eq!(*byte, 0);
        }
    }

    #[test]
    fn section_header_to_bytes_pads_correctly() {
        // ARRANGE
        // ACT
        let bytes = section_header_to_bytes(&ImageSectionHeader::default());
        // ASSERT
        assert_eq!(bytes.len(), core::mem::size_of::<ImageSectionHeader>());
        assert_eq!(byte_range(&bytes, 24..36), [0_u8; 12]);
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
            u32::from_le_bytes(byte_range(&bytes, 12..16).try_into().unwrap_or_default()),
            u32::MAX - 1
        );
        assert_eq!(
            u32::from_le_bytes(byte_range(&bytes, 16..20).try_into().unwrap_or_default()),
            u32::MAX - 2
        );
        assert_eq!(
            u32::from_le_bytes(byte_range(&bytes, 20..24).try_into().unwrap_or_default()),
            u32::MAX - 3
        );
    }

    #[test]
    fn build_headers_success() {
        // ARRANGE
        let metadata = create_test_metadata();
        let linux = vec![0_u8; 1024];
        let initrd = vec![0_u8; 2048];
        let cmdline = b"console=ttyS0";
        let data = Data {
            linux: &linux,
            initrd: &initrd,
            cmdline,
            dtb: None,
        };
        let (mut sections, _) = prepare(&data);

        // ACT
        let layout = build_headers(&metadata, &mut sections).unwrap_or_default();

        // ASSERT
        assert_eq!(layout.headers.len(), 3);
        assert_eq!(sections.len(), 3);
        assert_eq!(byte_range(&hdrs(&layout, 0).name, 0..8), b".cmdline");
        assert_eq!(byte_range(&hdrs(&layout, 1).name, 0..6), b".linux");
        assert_eq!(byte_range(&hdrs(&layout, 2).name, 0..7), b".initrd");
    }

    #[test]
    fn build_headers_sequential_offsets() {
        // ARRANGE
        let metadata = create_test_metadata();
        let linux = vec![0_u8; 512];
        let initrd = vec![0_u8; 512];
        let cmdline = b"cmd";
        let data = Data {
            linux: &linux,
            initrd: &initrd,
            cmdline,
            dtb: None,
        };
        let (mut sections, _) = prepare(&data);

        // ACT
        let _layout = build_headers(&metadata, &mut sections).unwrap_or_default();

        // ASSERT
        for i in 1..sections.len() {
            let prev = secs(&sections, i - 1);
            let curr = secs(&sections, i);
            assert!(curr.file_offset >= prev.file_offset + prev.size);
        }
        for section in &sections {
            assert!(section.file_offset > 0);
        }
    }

    #[test]
    fn build_headers_characteristics() {
        // ARRANGE
        let metadata = create_test_metadata();
        let linux = vec![0_u8; 100];
        let initrd = vec![0_u8; 100];
        let cmdline = b"test";
        let data = Data {
            linux: &linux,
            initrd: &initrd,
            cmdline,
            dtb: None,
        };
        let (mut sections, _) = prepare(&data);

        // ACT
        let layout = build_headers(&metadata, &mut sections).unwrap_or_default();

        // ASSERT
        assert_eq!(
            hdrs(&layout, 1).characteristics.get(LE),
            IMAGE_SCN_CNT_CODE | IMAGE_SCN_MEM_EXECUTE | IMAGE_SCN_MEM_READ
        );
        assert_eq!(
            hdrs(&layout, 0).characteristics.get(LE),
            IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ
        );
    }

    #[test]
    fn build_headers_max_virtual_end() {
        // ARRANGE
        let metadata = create_test_metadata();
        let linux = vec![0_u8; 1000];
        let initrd = vec![0_u8; 1000];
        let cmdline = b"test";
        let data = Data {
            linux: &linux,
            initrd: &initrd,
            cmdline,
            dtb: None,
        };
        let (mut sections, _) = prepare(&data);

        // ACT
        let layout = build_headers(&metadata, &mut sections).unwrap_or_default();

        // ASSERT
        assert!(layout.max_virtual_end > metadata.last_section_virtual_end);
    }

    #[test]
    fn build_headers_name_truncation() {
        // ARRANGE
        let metadata = create_test_metadata();
        let linux = vec![0_u8; 100];
        let initrd = vec![0_u8; 100];
        let cmdline = b"very_long_cmdline_name";
        let data = Data {
            linux: &linux,
            initrd: &initrd,
            cmdline,
            dtb: None,
        };
        let (mut sections, _) = prepare(&data);

        // ACT
        let layout = build_headers(&metadata, &mut sections).unwrap_or_default();

        // ASSERT
        assert!(
            byte_range(&hdrs(&layout, 0).name, 0..8)
                .iter()
                .any(|&byte| byte != 0)
        );
        assert_eq!(hdrs(&layout, 0).name.len(), 8);
    }

    #[test]
    fn build_headers_offsets_alignment() {
        // ARRANGE
        let metadata = create_test_metadata();
        let linux = vec![0_u8; 100];
        let initrd = vec![0_u8; 200];
        let cmdline = b"test";
        let data = Data {
            linux: &linux,
            initrd: &initrd,
            cmdline,
            dtb: None,
        };
        let (mut sections, _) = prepare(&data);

        // ACT
        let _layout = build_headers(&metadata, &mut sections).unwrap_or_default();

        // ASSERT
        let file_alignment = usize::try_from(metadata.file_alignment).unwrap_or_default();
        assert!(
            secs(&sections, 0)
                .file_offset
                .is_multiple_of(file_alignment)
        );
        for i in 1..sections.len() {
            let prev = secs(&sections, i - 1);
            let prev_end = prev.file_offset.checked_add(prev.size).unwrap();
            assert!(secs(&sections, i).file_offset >= prev_end);
        }
    }

    #[test]
    fn build_headers_rejects_sections_larger_than_u32() {
        // ARRANGE
        let metadata = create_test_metadata();
        let oversized_len = usize::try_from(u32::MAX)
            .unwrap_or_default()
            .saturating_add(1);
        // SAFETY: only used for length
        let huge = unsafe {
            core::slice::from_raw_parts(
                core::ptr::NonNull::<u8>::dangling().as_ptr(),
                oversized_len,
            )
        };
        let mut sections = make_sections(&[(".huge", huge)]);

        // ACT
        let result = build_headers(&metadata, &mut sections);

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(msg)) if msg.contains("section '.huge' too large")
        ));
    }

    #[test]
    fn build_headers_rejects_virtual_overflow() {
        // ARRANGE
        let metadata = PeMetadata {
            last_section_virtual_end: u32::MAX - 1024,
            ..create_test_metadata()
        };
        let mut sections = make_sections(&[(".ok", &[0_u8; 2048][..])]);

        // ACT
        let result = build_headers(&metadata, &mut sections);

        // ASSERT
        assert!(
            matches!(result, Err(YukiError::InvalidPeStructure(msg)) if msg.contains("section virtual end overflow"))
        );
    }

    #[test]
    fn build_headers_rejects_file_offset_overflow() {
        // ARRANGE
        let metadata = PeMetadata {
            last_section_file_end: u32::MAX - 256,
            ..create_test_metadata()
        };
        let mut sections = make_sections(&[(".ok", &[0_u8; 512][..])]);

        // ACT
        let result = build_headers(&metadata, &mut sections);

        // ASSERT
        assert!(
            matches!(result, Err(YukiError::InvalidPeStructure(msg)) if msg.contains("next section file offset overflow"))
        );
    }

    #[test]
    fn build_headers_next_virtual_address_matches_section_end() {
        // ARRANGE
        let metadata = create_test_metadata();
        let mut sections = make_sections(&[(".ok", &[0_u8; 1][..]), (".next", &[0_u8; 1][..])]);

        // ACT
        let layout = build_headers(&metadata, &mut sections).unwrap_or_default();

        // ASSERT
        let first_end = hdrs(&layout, 0)
            .virtual_address
            .get(LE)
            .saturating_add(binary::align_to(
                hdrs(&layout, 0).virtual_size.get(LE),
                metadata.section_alignment,
            ));
        assert_eq!(hdrs(&layout, 1).virtual_address.get(LE), first_end);
    }

    #[test]
    fn build_headers_with_dtb() {
        // ARRANGE
        let metadata = create_test_metadata();
        let linux = vec![0_u8; 1024];
        let initrd = vec![0_u8; 2048];
        let cmdline = b"console=ttyS0";
        let dtb = vec![0xd0, 0x0d, 0xfe, 0xed];
        let data = Data {
            linux: &linux,
            initrd: &initrd,
            cmdline,
            dtb: Some(&dtb),
        };
        let (mut sections, _) = prepare(&data);

        // ACT
        let layout = build_headers(&metadata, &mut sections).unwrap_or_default();

        // ASSERT
        assert_eq!(layout.headers.len(), 4);
        assert_eq!(sections.len(), 4);
        assert_eq!(byte_range(&hdrs(&layout, 0).name, 0..8), b".cmdline");
        assert_eq!(byte_range(&hdrs(&layout, 1).name, 0..4), b".dtb");
        assert_eq!(byte_range(&hdrs(&layout, 2).name, 0..6), b".linux");
        assert_eq!(byte_range(&hdrs(&layout, 3).name, 0..7), b".initrd");
    }

    #[test]
    fn build_headers_uses_exact_virtual_size() {
        // ARRANGE
        let metadata = create_test_metadata();
        let mut sections = make_sections(&[(".ok", &[0_u8; 16][..])]);

        // ACT
        let layout = build_headers(&metadata, &mut sections).unwrap_or_default();

        // ASSERT
        assert_eq!(hdrs(&layout, 0).virtual_size.get(LE), 16);
    }

    #[test]
    fn prepare_sections_produces_expected_order() {
        // ARRANGE
        let linux = b"linux";
        let initrd = b"initrd";
        let cmdline = b"quiet";
        let data = Data {
            linux,
            initrd,
            cmdline,
            dtb: None,
        };

        // ACT
        let (sections, _) = prepare(&data);

        // ASSERT
        assert_eq!(sections.len(), 3);
        assert_eq!(secs(&sections, 0).name, ".cmdline");
        assert_eq!(secs(&sections, 0).size, cmdline.len());
        assert_eq!(secs(&sections, 1).name, ".linux");
        assert_eq!(secs(&sections, 1).size, linux.len());
        assert_eq!(secs(&sections, 2).name, ".initrd");
        assert_eq!(secs(&sections, 2).size, initrd.len());
    }

    #[test]
    fn prepare_sections_with_dtb() {
        // ARRANGE
        let linux = b"linux";
        let initrd = b"initrd";
        let cmdline = b"quiet";
        let dtb = b"dtb_data";
        let data = Data {
            linux,
            initrd,
            cmdline,
            dtb: Some(dtb),
        };

        // ACT
        let (sections, _) = prepare(&data);

        // ASSERT
        assert_eq!(sections.len(), 4);
        assert_eq!(secs(&sections, 0).name, ".cmdline");
        assert_eq!(secs(&sections, 1).name, ".dtb");
        assert_eq!(secs(&sections, 1).size, dtb.len());
        assert_eq!(secs(&sections, 2).name, ".linux");
        assert_eq!(secs(&sections, 3).name, ".initrd");
    }

    #[test]
    fn build_headers_fills_file_offset() {
        // ARRANGE
        let metadata = create_test_metadata();
        let mut sections = make_sections(&[(".cmdline", b"console"), (".linux", b"kernel")]);

        // ACT
        let _layout = build_headers(&metadata, &mut sections).unwrap();

        // ASSERT
        assert!(
            secs(&sections, 0).file_offset > 0,
            "first section should have an offset"
        );
        assert!(
            secs(&sections, 1).file_offset
                > secs(&sections, 0).file_offset + secs(&sections, 0).size
        );
    }
}
