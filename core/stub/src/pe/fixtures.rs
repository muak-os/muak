//! Test PE file builder.

use object::pe::IMAGE_DIRECTORY_ENTRY_BASERELOC;

pub(crate) const FILE_ALIGN_U32: u32 = 0x200;

const FILE_ALIGN: usize = 0x200;
const NT_OFFSET: usize = 0x40;
const NT_OFFSET_U32: u32 = 0x40;
const OPT_OFF: usize = NT_OFFSET + 4 + 20;
const DD_OFF: usize = OPT_OFF + 112;
const SHDR_OFF: usize = DD_OFF + 16 * 8;

pub(crate) struct Builder {
    data: Vec<u8>,
    num_sections: u16,
}

impl Builder {
    pub(crate) fn new() -> Self {
        let hdr_size = FILE_ALIGN;
        let mut data = vec![0_u8; hdr_size];

        Self::write_bytes(&mut data, 0, b"MZ");
        Self::write_u32(&mut data, 0x3C, NT_OFFSET_U32);
        Self::write_bytes(&mut data, NT_OFFSET, b"PE");

        let file_header = Self::offset(NT_OFFSET, 4);
        Self::write_u16(&mut data, file_header, 0x8664);
        Self::write_u16(&mut data, Self::offset(file_header, 16), 0xF0);
        Self::write_u16(&mut data, Self::offset(file_header, 18), 0x0002);

        Self::write_u16(&mut data, OPT_OFF, 0x020B);
        Self::write_u32(&mut data, Self::offset(OPT_OFF, 16), FILE_ALIGN_U32);
        Self::write_u64(&mut data, Self::offset(OPT_OFF, 24), 0x0000_0000_0400_0000);
        Self::write_u32(&mut data, Self::offset(OPT_OFF, 32), FILE_ALIGN_U32);
        Self::write_u32(&mut data, Self::offset(OPT_OFF, 36), FILE_ALIGN_U32);
        Self::write_u16(&mut data, Self::offset(OPT_OFF, 44), 1);
        Self::write_u32(&mut data, Self::offset(OPT_OFF, 56), FILE_ALIGN_U32);
        Self::write_u32(&mut data, Self::offset(OPT_OFF, 60), FILE_ALIGN_U32);
        Self::write_u32(&mut data, Self::offset(OPT_OFF, 108), 16);

        Self {
            data,
            num_sections: 0,
        }
    }

    pub(crate) fn add_section(&mut self, name: [u8; 8], content: &[u8]) {
        let section_index = usize::from(self.num_sections);
        let raw_offset = Self::checked_mul(FILE_ALIGN, Self::offset(section_index, 1));
        let raw_size = Self::align_up(content.len(), FILE_ALIGN);

        let needed = Self::offset(raw_offset, raw_size);
        self.data.resize(self.data.len().max(needed), 0);
        let content_end = Self::offset(raw_offset, content.len());
        self.data
            .get_mut(raw_offset..content_end)
            .expect("section content range exists")
            .copy_from_slice(content);

        let section_header = Self::section_header_offset(self.num_sections);
        self.data
            .get_mut(section_header..Self::offset(section_header, 8))
            .expect("section header name range exists")
            .copy_from_slice(&name);
        Self::write_u32(
            &mut self.data,
            Self::offset(section_header, 8),
            Self::usize_to_u32(content.len()),
        );
        Self::write_u32(
            &mut self.data,
            Self::offset(section_header, 12),
            Self::usize_to_u32(raw_offset),
        );
        Self::write_u32(
            &mut self.data,
            Self::offset(section_header, 16),
            Self::usize_to_u32(raw_size),
        );
        Self::write_u32(
            &mut self.data,
            Self::offset(section_header, 20),
            Self::usize_to_u32(raw_offset),
        );

        self.num_sections = self
            .num_sections
            .checked_add(1)
            .expect("test PE section count overflow");

        let file_header = Self::offset(NT_OFFSET, 4);
        Self::write_u16(
            &mut self.data,
            Self::offset(file_header, 2),
            self.num_sections,
        );

        let new_image_size = Self::offset(raw_offset, raw_size);
        Self::write_u32(
            &mut self.data,
            Self::offset(OPT_OFF, 56),
            Self::usize_to_u32(new_image_size),
        );
    }

    pub(crate) fn set_last_virtual_size(&mut self, size: u32) {
        let section_header = self.last_section_header_offset();
        Self::write_u32(&mut self.data, Self::offset(section_header, 8), size);
    }

    pub(crate) fn set_last_ptr_relocs(&mut self, ptr: u32) {
        let section_header = self.last_section_header_offset();
        Self::write_u32(&mut self.data, Self::offset(section_header, 24), ptr);
    }

    pub(crate) fn set_dll_characteristics(&mut self, value: u16) {
        Self::write_u16(&mut self.data, Self::offset(OPT_OFF, 70), value);
    }

    pub(crate) fn set_major_image_version(&mut self, value: u16) {
        Self::write_u16(&mut self.data, Self::offset(OPT_OFF, 44), value);
    }

    pub(crate) fn set_entry_point(&mut self, rva: u32) {
        Self::write_u32(&mut self.data, Self::offset(OPT_OFF, 16), rva);
    }

    pub(crate) fn set_base_reloc_dir(&mut self, vaddr: u32, size: u32) {
        let offset = Self::offset(
            DD_OFF,
            Self::checked_mul(IMAGE_DIRECTORY_ENTRY_BASERELOC, 8),
        );
        Self::write_u32(&mut self.data, offset, vaddr);
        Self::write_u32(&mut self.data, Self::offset(offset, 4), size);
    }

    pub(crate) fn build(self) -> Vec<u8> {
        self.data
    }

    fn write_u16(buf: &mut [u8], offset: usize, value: u16) {
        Self::write_bytes(buf, offset, &value.to_le_bytes());
    }

    fn write_u32(buf: &mut [u8], offset: usize, value: u32) {
        Self::write_bytes(buf, offset, &value.to_le_bytes());
    }

    fn write_u64(buf: &mut [u8], offset: usize, value: u64) {
        Self::write_bytes(buf, offset, &value.to_le_bytes());
    }

    fn align_up(value: usize, align: usize) -> usize {
        let mask = align.checked_sub(1).expect("alignment is non-zero");
        value.checked_add(mask).expect("alignment overflow") & !mask
    }

    fn write_bytes(buf: &mut [u8], offset: usize, value: &[u8]) {
        let end = Self::offset(offset, value.len());
        buf.get_mut(offset..end)
            .expect("test PE write range exists")
            .copy_from_slice(value);
    }

    fn section_header_offset(section_index: u16) -> usize {
        Self::offset(SHDR_OFF, Self::checked_mul(usize::from(section_index), 40))
    }

    fn last_section_header_offset(&self) -> usize {
        let section_index = self
            .num_sections
            .checked_sub(1)
            .expect("test PE has at least one section");
        Self::section_header_offset(section_index)
    }

    fn offset(base: usize, offset: usize) -> usize {
        base.checked_add(offset).expect("test PE offset overflow")
    }

    fn checked_mul(left: usize, right: usize) -> usize {
        left.checked_mul(right).expect("test PE offset overflow")
    }

    fn usize_to_u32(value: usize) -> u32 {
        u32::try_from(value).expect("test PE value fits in u32")
    }
}
