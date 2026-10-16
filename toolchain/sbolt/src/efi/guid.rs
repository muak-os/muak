//! EFI GUIDs for Secure Boot.

use uefi::{Guid, guid};

/// X.509 certificate signature type.
pub const EFI_CERT_X509_GUID: Guid = guid!("a5c059a1-94e4-4aa7-87b5-ab155c2bf072");

/// PKCS#7 signature type for `WIN_CERTIFICATE`.
pub const EFI_CERT_TYPE_PKCS7_GUID: Guid = guid!("4aafd29d-68df-49ee-8aa9-347d375665a7");
