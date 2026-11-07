//! The block devices and partitions the kernel reports.

#![warn(missing_docs)]

pub mod device;
pub mod discover;
pub mod error;
pub mod holders;
pub mod inventory;
pub mod io;
pub mod mounts;
pub mod roots;
pub mod types;

pub(crate) mod attrs;
pub(crate) mod partitions;
pub(crate) mod probe;
