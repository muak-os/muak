//! Configuration constants for PE file structures and alignments.

/// PE section characteristic flag: section contains executable code.
pub const IMAGE_SCN_CNT_CODE: u32 = 0x0000_0020;

/// PE section characteristic flag: section contains initialized data.
pub const IMAGE_SCN_CNT_INITIALIZED_DATA: u32 = 0x0000_0040;

/// PE section characteristic flag: section is executable in memory.
pub const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;

/// PE section characteristic flag: section is readable in memory.
pub const IMAGE_SCN_MEM_READ: u32 = 0x4000_0000;

/// Byte offset within DOS header to the PE signature offset field.
pub const DOS_HEADER_PE_OFFSET: usize = 0x3C;

/// Size of the PE signature in bytes ("PE\0\0").
pub const PE_SIGNATURE_SIZE: usize = 4;

/// Byte offset within the optional header to the section alignment field.
pub const OPT_HEADER_SECTION_ALIGNMENT: usize = 32;

/// Byte offset within the optional header to the file alignment field.
pub const OPT_HEADER_FILE_ALIGNMENT: usize = 36;

/// Byte offset within the optional header to the `SizeOfImage` field.
pub const OPT_HEADER_SIZE_OF_IMAGE: usize = 56;

/// Byte offset within the COFF file header to the `NumberOfSections` field.
pub const COFF_NUMBER_OF_SECTIONS: usize = 2;

/// Maximum length of a PE section name in bytes.
pub const SECTION_NAME_MAX_LEN: usize = 8;
