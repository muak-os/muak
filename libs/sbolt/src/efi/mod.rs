//! EFI types and efivarfs interface.

mod authvar;
mod efivarfs;
mod enroll;
mod guid;
mod siglist;
mod time;

pub use authvar::sign_efi_variable;
pub use efivarfs::{
    get_db, get_kek, get_pk, get_secure_boot, get_setup_mode, is_efi_boot, is_efivarfs_available,
    mount_efivarfs,
};
pub use enroll::enroll_keys;
pub use guid::{
    EFI_CERT_TYPE_PKCS7_GUID, EFI_CERT_X509_GUID, EFI_GLOBAL_VARIABLE, EFI_IMAGE_SECURITY_DATABASE,
};
pub use siglist::{SignatureDatabase, build_x509_siglist};
pub use time::now as efi_time_now;
