//! Authenticated EFI variable signing

use uefi::Guid;
use uefi::runtime::{Time, VariableAttributes};
use x509_cert::Certificate;

use super::guid::EFI_CERT_TYPE_PKCS7_GUID;
use super::time::{now, to_bytes as time_to_bytes};
use crate::Result;
use crate::keys::Rsa2048Signer;
use crate::pkcs7::build_detached_signed_data;

const WIN_CERT_REVISION_2_0: u16 = 0x0200;
const WIN_CERT_TYPE_EFI_GUID: u16 = 0x0EF1;

/// Sign an EFI variable update with EFI_VARIABLE_AUTHENTICATION_2
pub fn sign_efi_variable(
    var_name: &str,
    vendor_guid: &Guid,
    attributes: VariableAttributes,
    content: &[u8],
    signer: &Rsa2048Signer,
    certificate: &Certificate,
) -> Result<Vec<u8>> {
    let timestamp = now();

    let descriptor = build_descriptor(var_name, vendor_guid, attributes, &timestamp, content);

    let pkcs7_der = build_detached_signed_data(&descriptor, signer, certificate)?;

    let win_cert = build_win_certificate(&pkcs7_der);

    let mut payload = Vec::with_capacity(4 + 16 + win_cert.len() + content.len());
    payload.extend_from_slice(&attributes.bits().to_le_bytes());
    payload.extend_from_slice(&time_to_bytes(&timestamp));
    payload.extend_from_slice(&win_cert);
    payload.extend_from_slice(content);

    Ok(payload)
}

/// Build the descriptor that gets signed per UEFI spec 8.2.2
fn build_descriptor(
    var_name: &str,
    vendor_guid: &Guid,
    attributes: VariableAttributes,
    timestamp: &Time,
    content: &[u8],
) -> Vec<u8> {
    let mut desc = Vec::new();

    for c in var_name.encode_utf16() {
        desc.extend_from_slice(&c.to_le_bytes());
    }

    desc.extend_from_slice(&vendor_guid.to_bytes());
    desc.extend_from_slice(&attributes.bits().to_le_bytes());
    desc.extend_from_slice(&time_to_bytes(timestamp));
    desc.extend_from_slice(content);

    desc
}

/// Build WIN_CERTIFICATE_UEFI_GUID structure.
fn build_win_certificate(pkcs7_der: &[u8]) -> Vec<u8> {
    let header_size = 4 + 2 + 2 + 16; // 24 bytes
    let total_size = header_size + pkcs7_der.len();

    let mut result = Vec::with_capacity(total_size);

    result.extend_from_slice(&(total_size as u32).to_le_bytes());
    result.extend_from_slice(&WIN_CERT_REVISION_2_0.to_le_bytes());
    result.extend_from_slice(&WIN_CERT_TYPE_EFI_GUID.to_le_bytes());
    result.extend_from_slice(&EFI_CERT_TYPE_PKCS7_GUID.to_bytes());
    result.extend_from_slice(pkcs7_der);

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use uefi::guid;

    use crate::keys::{
        Rsa2048Signer, generate_db_certificate, generate_kek_certificate, generate_pk_certificate,
    };

    fn test_signer_and_cert() -> (Rsa2048Signer, Certificate) {
        let (pk_signer, pk_cert) = generate_pk_certificate("Test PK").expect("generate PK cert");
        let (kek_signer, kek_cert) =
            generate_kek_certificate("Test KEK", &pk_signer, &pk_cert).expect("generate KEK cert");
        let (db_signer, db_cert) =
            generate_db_certificate("Test DB", &kek_signer, &kek_cert).expect("generate DB cert");
        (db_signer, db_cert)
    }

    #[test]
    fn build_descriptor_layout() {
        let var_name = "db";
        let vendor = guid!("d719b2cb-3d3a-4596-a3bc-dad00e67656f");
        let attrs = VariableAttributes::NON_VOLATILE
            | VariableAttributes::BOOTSERVICE_ACCESS
            | VariableAttributes::RUNTIME_ACCESS
            | VariableAttributes::TIME_BASED_AUTHENTICATED_WRITE_ACCESS;

        let timestamp = uefi::runtime::Time::new(uefi::runtime::TimeParams {
            year: 2025,
            month: 1,
            day: 15,
            hour: 12,
            minute: 0,
            second: 0,
            nanosecond: 0,
            time_zone: Some(0),
            daylight: uefi::runtime::Daylight::empty(),
        })
        .expect("valid time");

        let content = b"test-content";

        let desc = build_descriptor(var_name, &vendor, attrs, &timestamp, content);

        let mut offset = 0;

        // UTF-16LE var_name: "db" -> [0x64, 0x00, 0x62, 0x00]
        let name_utf16: Vec<u16> = var_name.encode_utf16().collect();
        let name_len_bytes = name_utf16.len() * 2;
        for ch in &name_utf16 {
            let got = u16::from_le_bytes([desc[offset], desc[offset + 1]]);
            assert_eq!(got, *ch, "UTF-16LE mismatch at offset {offset}");
            offset += 2;
        }

        // 16-byte vendor GUID
        assert_eq!(&desc[offset..offset + 16], &vendor.to_bytes());
        offset += 16;

        // 4-byte attributes
        let got_attrs = u32::from_le_bytes(desc[offset..offset + 4].try_into().expect("4 bytes"));
        assert_eq!(got_attrs, attrs.bits());
        offset += 4;

        // 16-byte EFI_TIME
        let time_bytes = super::time_to_bytes(&timestamp);
        assert_eq!(&desc[offset..offset + 16], &time_bytes);
        offset += 16;

        // Content
        assert_eq!(&desc[offset..], content);

        // Total length
        assert_eq!(desc.len(), name_len_bytes + 16 + 4 + 16 + content.len());
    }

    #[test]
    fn build_win_certificate_layout() {
        let fake_pkcs7 = b"fake-pkcs7-signature-data";
        let wc = build_win_certificate(fake_pkcs7);

        let expected_total = 24 + fake_pkcs7.len();
        assert_eq!(wc.len(), expected_total);

        // [0..4] total size
        let total_size = u32::from_le_bytes(wc[0..4].try_into().expect("4 bytes"));
        assert_eq!(total_size as usize, expected_total);

        // [4..6] revision = 0x0200
        let revision = u16::from_le_bytes(wc[4..6].try_into().expect("2 bytes"));
        assert_eq!(revision, 0x0200);

        // [6..8] cert type = 0x0EF1
        let cert_type = u16::from_le_bytes(wc[6..8].try_into().expect("2 bytes"));
        assert_eq!(cert_type, 0x0EF1);

        // [8..24] EFI_CERT_TYPE_PKCS7_GUID
        assert_eq!(&wc[8..24], &EFI_CERT_TYPE_PKCS7_GUID.to_bytes());

        // [24..] pkcs7 data
        assert_eq!(&wc[24..], fake_pkcs7);
    }

    #[test]
    fn sign_efi_variable_structural_correctness() {
        let (signer, cert) = test_signer_and_cert();
        let var_name = "db";
        let vendor = guid!("d719b2cb-3d3a-4596-a3bc-dad00e67656f");
        let attrs = VariableAttributes::NON_VOLATILE
            | VariableAttributes::BOOTSERVICE_ACCESS
            | VariableAttributes::RUNTIME_ACCESS
            | VariableAttributes::TIME_BASED_AUTHENTICATED_WRITE_ACCESS;
        let content = b"test-siglist-data";

        let payload = sign_efi_variable(var_name, &vendor, attrs, content, &signer, &cert)
            .expect("sign_efi_variable should succeed");

        // [0..4] attributes
        let got_attrs = u32::from_le_bytes(payload[0..4].try_into().expect("4 bytes"));
        assert_eq!(got_attrs, attrs.bits());

        // [4..20] timestamp (16 bytes, should be non-zero)
        let timestamp_bytes = &payload[4..20];
        assert!(
            timestamp_bytes.iter().any(|&b| b != 0),
            "timestamp should not be all zeros"
        );
        // Year should be reasonable (stored as u16 LE at bytes 0..2 of timestamp)
        let year = u16::from_le_bytes([timestamp_bytes[0], timestamp_bytes[1]]);
        assert!(year >= 2024 && year <= 2100, "year {year} out of range");

        // [20..24] WIN_CERT dwLength
        let win_cert_size =
            u32::from_le_bytes(payload[20..24].try_into().expect("4 bytes")) as usize;
        assert!(win_cert_size >= 24, "WIN_CERT must be at least 24 bytes");

        // [24..26] revision
        let revision = u16::from_le_bytes(payload[24..26].try_into().expect("2 bytes"));
        assert_eq!(revision, 0x0200);

        // [26..28] cert type
        let cert_type = u16::from_le_bytes(payload[26..28].try_into().expect("2 bytes"));
        assert_eq!(cert_type, 0x0EF1);

        // [28..44] PKCS7 GUID
        assert_eq!(&payload[28..44], &EFI_CERT_TYPE_PKCS7_GUID.to_bytes());

        // Payload ends with content
        assert_eq!(
            &payload[payload.len() - content.len()..],
            content,
            "payload must end with the content bytes"
        );

        // Total = 4 (attrs) + 16 (timestamp) + win_cert_size + content.len()
        assert_eq!(payload.len(), 4 + 16 + win_cert_size + content.len());
    }
}
