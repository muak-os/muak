//! EFI System Partition manifest types and helpers.

#![warn(missing_docs)]

extern crate alloc;

pub mod arch;
pub mod error;
pub mod image;
pub mod layout;
pub mod path;

use fatfs::types;

/// Metadata for a file in the ESP (path and size).
pub type FileMeta<'a> = types::FileMeta<'a>;

/// The EFI System Partition type GUID (C12A7328-F81F-11D2-BA4B-00A0C93EC93B).
pub const EFI_GUID: [u8; 16] = [
    0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b,
];
