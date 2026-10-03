//! Create a Unified Kernel Images (UKI) to boot on UEFI systems.

#![warn(missing_docs)]

pub mod error;
mod io;
pub mod layout;
pub mod pe;
pub mod prepare;
pub mod probe;
pub mod write;
