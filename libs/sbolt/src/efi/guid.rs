//! EFI GUIDs for Secure Boot

use uefi::{Guid, guid};

/// X.509 certificate signature type
pub const EFI_CERT_X509_GUID: Guid = guid!("a5c059a1-94e4-4aa7-87b5-ab155c2bf072");

/// PKCS#7 signature type for WIN_CERTIFICATE
pub const EFI_CERT_TYPE_PKCS7_GUID: Guid = guid!("4aafd29d-68df-49ee-8aa9-347d375665a7");

/// Namespace for PK, KEK, SetupMode, SecureBoot
pub const EFI_GLOBAL_VARIABLE: Guid = guid!("8be4df61-93ca-11d2-aa0d-00e098032b8c");

/// Namespace for db/dbx
pub const EFI_IMAGE_SECURITY_DATABASE: Guid = guid!("d719b2cb-3d3a-4596-a3bc-dad00e67656f");
