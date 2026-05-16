//! EFI System Partition manifest types and helpers.

mod collect;
mod error;
mod format;
mod image;
mod model;
mod path;
mod populate;

pub use collect::collect_tree;
pub use error::EspError;
pub use format::format;
pub use image::build;
pub use model::{Arch, EspFile, EspSpec};
pub use populate::populate;

/// EFI System Partition type GUID (C12A7328-F81F-11D2-BA4B-00A0C93EC93B).
pub const EFI_GUID: [u8; 16] = [
    0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b,
];
