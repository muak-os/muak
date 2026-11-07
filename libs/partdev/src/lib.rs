//! Read, write it, and keep the kernel's view in sync of the partition table.

#![warn(missing_docs)]

pub mod blkpg;
pub mod clear;
pub mod error;
pub mod manage;
pub mod table;
