//! FAT32 filesystem image builder.

#![warn(missing_docs)]

mod boot;
pub mod builder;
mod dir;
pub mod error;
mod layout;
mod name;
mod table;
mod tree;
pub mod types;
