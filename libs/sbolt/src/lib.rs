//! `sbolt` secure boot signing support for UEFI.
//!
//! This crate provides functionality for:
//! - Generating Secure Boot key hierarchies (PK, KEK, db)
//! - Signing PE/COFF binaries with Authenticode signatures
//! - Managing EFI variables via a platform backend
//! - Enrolling keys to UEFI firmware

#![warn(missing_docs)]

pub mod efi;
pub mod error;
pub mod keys;
pub mod pe;
mod pkcs7;
mod platform;
