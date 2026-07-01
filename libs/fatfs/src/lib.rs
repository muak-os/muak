//! FAT12/FAT16/FAT32 filesystem image builder.

#![warn(missing_docs)]

mod boot;
pub mod builder;
mod dir;
pub mod error;
mod table;
pub mod types;
