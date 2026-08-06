//! Create a Unified Kernel Images (UKI) to boot on UEFI systems.

#![warn(missing_docs)]

#[cfg(feature = "cli")]
pub mod cli;
pub mod error;
mod io;
pub mod layout;
pub mod pe;
pub mod prepare;
pub mod probe;
pub mod write;
