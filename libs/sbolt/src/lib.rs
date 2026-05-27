//! `sbolt` secure boot signing support for UEFI.
//!
//! This crate provides functionality for:
//! - Generating Secure Boot key hierarchies (PK, KEK, db)
//! - Signing PE/COFF binaries with Authenticode signatures
//! - Managing EFI variables via efivarfs
//! - Enrolling keys to UEFI firmware

#![expect(
    clippy::pub_use,
    reason = "The crate intentionally exposes a flat public API at `sbolt::...`"
)]

mod efi;
mod error;
mod keys;
mod pe;
mod pkcs7;

pub use crate::efi::{
    EFI_CERT_TYPE_PKCS7_GUID, EFI_CERT_X509_GUID, EFI_GLOBAL_VARIABLE, EFI_IMAGE_SECURITY_DATABASE,
    SignatureDatabase, build_x509_siglist, efi_time_now, enroll_keys, get_db, get_kek, get_pk,
    get_secure_boot, get_setup_mode, is_efi_boot, is_efivarfs_available, mount_efivarfs,
    sign_efi_variable,
};
pub use error::{Result, SboltError as Error};
pub use keys::{
    KeyHierarchy, KeyPair, KeyType, Rsa2048Signature, Rsa2048Signer, generate_db_certificate,
    generate_kek_certificate, generate_pk_certificate, load_key_hierarchy, load_keypair,
    save_key_hierarchy,
};
pub use pe::{compute_hash, sign};
