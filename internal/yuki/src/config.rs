pub const IMAGE_SCN_CNT_CODE: u32 = 0x0000_0020;
pub const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;
pub const IMAGE_SCN_MEM_READ: u32 = 0x4000_0000;

pub const DOS_HEADER_PE_OFFSET: usize = 0x3C;
pub const PE_SIGNATURE_SIZE: usize = 4;

pub const OPT_HEADER_SECTION_ALIGNMENT: usize = 32;
pub const OPT_HEADER_FILE_ALIGNMENT: usize = 36;
pub const OPT_HEADER_SIZE_OF_IMAGE: usize = 56;

pub const COFF_NUMBER_OF_SECTIONS: usize = 2;

pub const SECTION_NAME_MAX_LEN: usize = 8;
