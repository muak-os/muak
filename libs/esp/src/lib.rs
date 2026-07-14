//! EFI System Partition manifest types and helpers.

#![warn(missing_docs)]

extern crate alloc;

pub mod arch;
pub mod builder;
pub mod error;
pub mod path;
pub mod populate;

use fatfs::types;

/// Metadata for a file in the ESP (path and size).
pub type FileMeta<'a> = types::FileMeta<'a>;
