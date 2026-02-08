//! sbolt - Secure Boot signing tool for UEFI
//!
//! This crate provides functionality for:
//! - Generating Secure Boot key hierarchies (PK, KEK, db)
//! - Signing PE/COFF binaries with Authenticode signatures
//! - Managing EFI variables via efivarfs
//! - Enrolling keys to UEFI firmware

pub mod efi;
pub mod error;
pub mod keys;
pub mod pe;
mod pkcs7;

pub use error::{Error, Result};
