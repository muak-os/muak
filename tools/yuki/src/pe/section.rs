//! PE section creation and embedding.

use object::LittleEndian as LE;
use object::pe::ImageSectionHeader;

use super::parse::{self, Metadata};
use crate::align;
use crate::error::{Result, YukiError};

const SECTION_NAME_MAX_LEN: usize = 8;
const SECTION_NAME_OFFSET: usize = 0;
const SECTION_VIRTUAL_SIZE_OFFSET: usize = 8;
const SECTION_VIRTUAL_ADDRESS_OFFSET: usize = 12;
const SECTION_SIZE_OF_RAW_DATA_OFFSET: usize = 16;
const SECTION_POINTER_TO_RAW_DATA_OFFSET: usize = 20;
const SECTION_RESERVED_OFFSET: usize = 24;
const SECTION_RESERVED_SIZE: usize = 12;
const SECTION_CHARACTERISTICS_OFFSET: usize = 36;

const IMAGE_SCN_CNT_CODE: u32 = 0x0000_0020;
const IMAGE_SCN_CNT_INITIALIZED_DATA: u32 = 0x0000_0040;
const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;
const IMAGE_SCN_MEM_READ: u32 = 0x4000_0000;

/// A PE section in the output UKI image, with its file offset and size.
#[derive(Debug, Clone)]
pub struct Section {
    /// PE section name.
    pub name: &'static str,
    /// File offset of the section data within the output PE image.
    pub file_offset: usize,
    /// Size of the section data in bytes.
    pub size: usize,
    /// SHA-256 checksum of the section data.
    pub checksum: [u8; 32],
}

#[derive(Debug)]
pub(crate) struct Table {
    pub(crate) sections: Vec<Section>,
    pub(crate) headers: Vec<ImageSectionHeader>,
    pub(crate) current_file_offset: u32,
    pub(crate) current_virtual_address: u32,
    pub(crate) max_virtual_end: u32,
    pub(crate) file_alignment: u32,
    pub(crate) section_alignment: u32,
}

impl Table {
    pub fn new(metadata: &Metadata) -> Self {
        Self {
            sections: Vec::new(),
            headers: Vec::new(),
            current_file_offset: align::to(metadata.last_section_file_end, metadata.file_alignment),
            current_virtual_address: align::to(
                metadata.last_section_virtual_end,
                metadata.section_alignment,
            ),
            max_virtual_end: metadata.last_section_virtual_end,
            file_alignment: metadata.file_alignment,
            section_alignment: metadata.section_alignment,
        }
    }

    pub fn finalize_section(&mut self, name: &'static str, size: usize) -> Result<()> {
        let virtual_size = u32::try_from(size).map_err(|_conversion_error| {
            YukiError::InvalidPeStructure(format!("section '{name}' too large"))
        })?;

        let size_of_raw_data = align::to(virtual_size, self.file_alignment);
        let aligned_virtual_size = align::to(virtual_size, self.section_alignment);

        let Ok(section_file_offset) = usize::try_from(self.current_file_offset) else {
            return Err(YukiError::InvalidPeStructure(
                "section file offset overflow".to_owned(),
            ));
        };

        self.sections.push(Section {
            name,
            file_offset: section_file_offset,
            size,
            checksum: [0; 32],
        });
        self.headers.push(build_header(
            name,
            virtual_size,
            size_of_raw_data,
            self.current_file_offset,
            self.current_virtual_address,
        ));

        let section_virtual_end = self
            .current_virtual_address
            .checked_add(aligned_virtual_size)
            .ok_or_else(|| {
                YukiError::InvalidPeStructure("section virtual end overflow".to_owned())
            })?;

        self.max_virtual_end = self.max_virtual_end.max(section_virtual_end);
        self.current_file_offset = self
            .current_file_offset
            .checked_add(size_of_raw_data)
            .ok_or_else(|| {
                YukiError::InvalidPeStructure("next section file offset overflow".to_owned())
            })?;
        self.current_virtual_address = section_virtual_end;

        Ok(())
    }

    pub fn max_virtual_end(&self) -> u32 {
        self.max_virtual_end
    }
}

fn build_header(
    name: &str,
    virtual_size: u32,
    size_of_raw_data: u32,
    pointer_to_raw_data: u32,
    virtual_address: u32,
) -> ImageSectionHeader {
    let mut header = ImageSectionHeader::default();
    for (dst, src) in header
        .name
        .iter_mut()
        .take(SECTION_NAME_MAX_LEN)
        .zip(name.as_bytes().iter().take(SECTION_NAME_MAX_LEN))
    {
        *dst = *src;
    }
    header.virtual_size.set(LE, virtual_size);
    header.virtual_address.set(LE, virtual_address);
    header.size_of_raw_data.set(LE, size_of_raw_data);
    header.pointer_to_raw_data.set(LE, pointer_to_raw_data);
    header.characteristics.set(
        LE,
        if name == ".linux" {
            IMAGE_SCN_CNT_CODE | IMAGE_SCN_MEM_EXECUTE | IMAGE_SCN_MEM_READ
        } else {
            IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ
        },
    );

    header
}

pub(crate) fn validate_size(byte_len: u64, name: &'static str) -> Result<usize> {
    let Ok(len) = usize::try_from(byte_len) else {
        return Err(YukiError::InvalidPeStructure(format!(
            "section '{name}' length exceeds usize"
        )));
    };
    if u32::try_from(len).is_err() {
        return Err(YukiError::InvalidPeStructure(format!(
            "section '{name}' too large"
        )));
    }

    Ok(len)
}

pub(crate) fn build_table(
    metadata: &Metadata,
    stub_len: u64,
    has_dtb: bool,
    sizes: &[(&'static str, Option<u64>)],
) -> Result<(Table, u64)> {
    let count = new_section_count(has_dtb);
    if usize::from(metadata.existing_section_count).saturating_add(usize::from(count))
        > usize::from(u16::MAX)
    {
        return Err(YukiError::TooManySections);
    }
    parse::validate_section_header_capacity(metadata, usize::from(count))?;

    let mut table = Table::new(metadata);
    let Ok(stub_file_off) = u32::try_from(stub_len) else {
        return Err(YukiError::InvalidPeStructure(
            "stub file offset overflow".to_owned(),
        ));
    };
    table.current_file_offset = table.current_file_offset.max(stub_file_off);

    for &(name, maybe_len) in sizes {
        let Some(len) = maybe_len else {
            continue;
        };
        table.finalize_section(name, validate_size(len, name)?)?;
    }

    let Some(first) = table.sections.first() else {
        return Err(YukiError::InvalidPeStructure(
            "missing generated sections".to_owned(),
        ));
    };
    let Ok(gap_start) = u64::try_from(first.file_offset) else {
        return Err(YukiError::InvalidPeStructure(
            "first section offset overflow".to_owned(),
        ));
    };

    Ok((table, gap_start))
}

pub(crate) fn count(has_dtb: bool) -> u16 {
    new_section_count(has_dtb)
}

fn new_section_count(has_dtb: bool) -> u16 {
    3_u16.saturating_add(u16::from(has_dtb))
}

pub(crate) fn header_to_bytes(
    header: &ImageSectionHeader,
) -> [u8; core::mem::size_of::<ImageSectionHeader>()] {
    let mut bytes = [0_u8; core::mem::size_of::<ImageSectionHeader>()];

    bytes[SECTION_NAME_OFFSET..SECTION_NAME_OFFSET + 8].copy_from_slice(&header.name);
    bytes[SECTION_VIRTUAL_SIZE_OFFSET..SECTION_VIRTUAL_SIZE_OFFSET + 4]
        .copy_from_slice(&header.virtual_size.get(LE).to_le_bytes());
    bytes[SECTION_VIRTUAL_ADDRESS_OFFSET..SECTION_VIRTUAL_ADDRESS_OFFSET + 4]
        .copy_from_slice(&header.virtual_address.get(LE).to_le_bytes());
    bytes[SECTION_SIZE_OF_RAW_DATA_OFFSET..SECTION_SIZE_OF_RAW_DATA_OFFSET + 4]
        .copy_from_slice(&header.size_of_raw_data.get(LE).to_le_bytes());
    bytes[SECTION_POINTER_TO_RAW_DATA_OFFSET..SECTION_POINTER_TO_RAW_DATA_OFFSET + 4]
        .copy_from_slice(&header.pointer_to_raw_data.get(LE).to_le_bytes());
    bytes[SECTION_RESERVED_OFFSET..SECTION_RESERVED_OFFSET + SECTION_RESERVED_SIZE].fill(0);
    bytes[SECTION_CHARACTERISTICS_OFFSET..SECTION_CHARACTERISTICS_OFFSET + 4]
        .copy_from_slice(&header.characteristics.get(LE).to_le_bytes());

    bytes
}

#[cfg(test)]
mod tests {
    use object::LittleEndian as LE;
    use object::pe::ImageSectionHeader;

    use super::*;
    use crate::error::YukiError;

    fn byte_range(bytes: &[u8], range: core::ops::Range<usize>) -> &[u8] {
        bytes.get(range).unwrap()
    }

    fn create_test_metadata() -> Metadata {
        Metadata {
            file_header_offset: 0,
            optional_header_offset: 0,
            section_table_offset: 0,
            size_of_headers: 512,
            section_alignment: 4096,
            file_alignment: 512,
            last_section_file_end: 512,
            last_section_virtual_end: 4096,
            existing_section_count: 1,
        }
    }

    fn assert_section_header(
        header: &ImageSectionHeader,
        name: &[u8],
        virtual_size: u32,
        virtual_address: u32,
        size_of_raw_data: u32,
        pointer_to_raw_data: u32,
        characteristics: u32,
    ) {
        assert_eq!(byte_range(&header.name, 0..name.len()), name);
        assert_eq!(header.virtual_size.get(LE), virtual_size);
        assert_eq!(header.virtual_address.get(LE), virtual_address);
        assert_eq!(header.size_of_raw_data.get(LE), size_of_raw_data);
        assert_eq!(header.pointer_to_raw_data.get(LE), pointer_to_raw_data);
        assert_eq!(header.characteristics.get(LE), characteristics);
    }

    #[test]
    fn layout_state_finalize_section_basic() {
        // ARRANGE
        let metadata = create_test_metadata();
        let mut state = Table::new(&metadata);

        // ACT
        state.finalize_section(".linux", 100).unwrap();

        // ASSERT
        assert_eq!(state.sections.len(), 1);
        assert_eq!(state.sections.first().unwrap().name, ".linux");
        assert_eq!(state.sections.first().unwrap().size, 100);
        assert_eq!(state.sections.first().unwrap().file_offset, 512);
        assert_eq!(state.sections.first().unwrap().checksum, [0_u8; 32]);

        assert_eq!(state.headers.len(), 1);
        assert_section_header(
            state.headers.first().unwrap(),
            b".linux",
            100,
            4096,
            512,
            512,
            IMAGE_SCN_CNT_CODE | IMAGE_SCN_MEM_EXECUTE | IMAGE_SCN_MEM_READ,
        );
    }

    #[test]
    fn layout_state_sequential_offsets() {
        // ARRANGE
        let metadata = create_test_metadata();
        let mut state = Table::new(&metadata);

        // ACT
        state.finalize_section(".cmdline", 10).unwrap();
        state.finalize_section(".linux", 200).unwrap();
        state.finalize_section(".initrd", 300).unwrap();

        // ASSERT
        assert_eq!(state.sections.len(), 3);
        for i in 1..state.sections.len() {
            let prev = state.sections.get(i - 1).unwrap();
            let curr = state.sections.get(i).unwrap();
            assert!(
                curr.file_offset >= prev.file_offset + prev.size,
                "section {} should start after section {}",
                curr.name,
                prev.name
            );
        }
        assert!(state.sections.first().unwrap().file_offset > 0);
    }

    #[test]
    fn layout_state_characteristics() {
        // ARRANGE
        let metadata = create_test_metadata();
        let mut state = Table::new(&metadata);

        // ACT
        state.finalize_section(".cmdline", 10).unwrap();
        state.finalize_section(".linux", 100).unwrap();

        // ASSERT
        assert_eq!(
            state.headers.first().unwrap().characteristics.get(LE),
            IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ
        );
        assert_eq!(
            state.headers.get(1).unwrap().characteristics.get(LE),
            IMAGE_SCN_CNT_CODE | IMAGE_SCN_MEM_EXECUTE | IMAGE_SCN_MEM_READ
        );
    }

    #[test]
    fn layout_state_max_virtual_end() {
        // ARRANGE
        let metadata = create_test_metadata();
        let mut state = Table::new(&metadata);

        // ACT
        state.finalize_section(".cmdline", 10).unwrap();
        state.finalize_section(".linux", 1000).unwrap();

        // ASSERT
        assert!(state.max_virtual_end() > metadata.last_section_virtual_end);
    }

    #[test]
    fn layout_state_name_truncation() {
        // ARRANGE
        let metadata = create_test_metadata();
        let mut state = Table::new(&metadata);

        // ACT
        state
            .finalize_section("very_long_cmdline_name", 10)
            .unwrap();

        // ASSERT
        assert!(
            state
                .headers
                .first()
                .unwrap()
                .name
                .iter()
                .any(|&byte| byte != 0),
            "name should have bytes"
        );
        assert_eq!(state.headers.first().unwrap().name.len(), 8);
    }

    #[test]
    fn layout_state_alignment() {
        // ARRANGE
        let metadata = create_test_metadata();
        let mut state = Table::new(&metadata);

        // ACT
        state.finalize_section(".cmdline", 10).unwrap();
        state.finalize_section(".linux", 100).unwrap();

        // ASSERT
        let file_alignment = usize::try_from(metadata.file_alignment).unwrap();
        assert!(
            state
                .sections
                .first()
                .unwrap()
                .file_offset
                .is_multiple_of(file_alignment)
        );
    }

    #[test]
    fn layout_state_rejects_oversized_section() {
        // ARRANGE
        let metadata = create_test_metadata();
        let mut state = Table::new(&metadata);
        let oversized = usize::try_from(u32::MAX).unwrap().saturating_add(1);

        // ACT
        let result = state.finalize_section(".huge", oversized);

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(msg)) if msg.contains("section '.huge' too large")
        ));
    }

    #[test]
    fn layout_state_rejects_virtual_overflow() {
        // ARRANGE
        let metadata = Metadata {
            last_section_virtual_end: u32::MAX - 1024,
            ..create_test_metadata()
        };
        let mut state = Table::new(&metadata);

        // ACT
        let result = state.finalize_section(".ok", 2048);

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(msg)) if msg.contains("section virtual end overflow")
        ));
    }

    #[test]
    fn layout_state_rejects_file_offset_overflow() {
        // ARRANGE
        let metadata = Metadata {
            last_section_file_end: u32::MAX - 256,
            ..create_test_metadata()
        };
        let mut state = Table::new(&metadata);

        // ACT
        let result = state.finalize_section(".ok", 512);

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(msg)) if msg.contains("next section file offset overflow")
        ));
    }

    #[test]
    fn layout_state_virtual_address_is_sequential() {
        // ARRANGE
        let metadata = create_test_metadata();
        let mut state = Table::new(&metadata);

        // ACT
        state.finalize_section(".first", 1).unwrap();
        state.finalize_section(".second", 1).unwrap();

        // ASSERT
        let first_end = state
            .headers
            .first()
            .unwrap()
            .virtual_address
            .get(LE)
            .saturating_add(align::to(
                state.headers.first().unwrap().virtual_size.get(LE),
                metadata.section_alignment,
            ));
        assert_eq!(
            state.headers.get(1).unwrap().virtual_address.get(LE),
            first_end
        );
    }

    #[test]
    fn layout_state_with_dtb_order() {
        // ARRANGE
        let metadata = create_test_metadata();
        let mut state = Table::new(&metadata);

        // ACT
        state.finalize_section(".cmdline", 10).unwrap();
        state.finalize_section(".dtb", 100).unwrap();
        state.finalize_section(".linux", 200).unwrap();
        state.finalize_section(".initrd", 300).unwrap();

        assert_eq!(state.sections.len(), 4);
        assert_eq!(state.sections.first().unwrap().name, ".cmdline");
        assert_eq!(state.sections.get(1).unwrap().name, ".dtb");
        assert_eq!(state.sections.get(2).unwrap().name, ".linux");
        assert_eq!(state.sections.get(3).unwrap().name, ".initrd");
    }

    #[test]
    fn layout_state_exact_virtual_size() {
        // ARRANGE
        let metadata = create_test_metadata();
        let mut state = Table::new(&metadata);

        // ACT
        state.finalize_section(".ok", 16).unwrap();

        // ASSERT

        assert_eq!(state.headers.first().unwrap().virtual_size.get(LE), 16);
    }

    #[test]
    fn layout_state_file_offset_increases() {
        // ARRANGE
        let metadata = create_test_metadata();
        let mut state = Table::new(&metadata);

        // ACT
        state.finalize_section(".cmdline", 10).unwrap();
        state.finalize_section(".linux", 50).unwrap();

        assert!(
            state.sections.first().unwrap().file_offset > 0,
            "first section should have an offset"
        );
        assert!(
            state.sections.get(1).unwrap().file_offset
                > state.sections.first().unwrap().file_offset
                    + state.sections.first().unwrap().size
        );
    }

    #[test]
    fn header_to_bytes_basic() {
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

        let bytes = header_to_bytes(&header);

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
    fn header_to_bytes_all_zeros() {
        // ARRANGE & ACT
        let bytes = header_to_bytes(&ImageSectionHeader::default());

        // ASSERT
        for byte in byte_range(&bytes, 24..36) {
            assert_eq!(*byte, 0);
        }
    }

    #[test]
    fn header_to_bytes_pads_correctly() {
        // ARRANGE & ACT
        let bytes = header_to_bytes(&ImageSectionHeader::default());

        // ASSERT
        assert_eq!(bytes.len(), core::mem::size_of::<ImageSectionHeader>());
        assert_eq!(byte_range(&bytes, 24..36), [0_u8; 12]);
    }

    #[test]
    fn header_to_bytes_big_values() {
        // ARRANGE
        let mut header = ImageSectionHeader::default();
        header.virtual_size.set(LE, u32::MAX);
        header.virtual_address.set(LE, u32::MAX - 1);
        header.size_of_raw_data.set(LE, u32::MAX - 2);
        header.pointer_to_raw_data.set(LE, u32::MAX - 3);

        // ACT
        let bytes = header_to_bytes(&header);

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
    fn validate_size_rejects_overflow() {
        // ARRANGE
        let oversized = u64::from(u32::MAX).saturating_add(1);

        // ACT
        let result = validate_size(oversized, ".big");

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(msg))
                if msg.contains("section '.big' too large")
        ));
    }
}
