//! Authenticated EFI variable signing.

use uefi::Guid;
use uefi::runtime::{Time, VariableAttributes};
use x509_cert::Certificate;

use super::guid::EFI_CERT_TYPE_PKCS7_GUID;
use super::time::{now, to_bytes as time_to_bytes};
use crate::error::{Result, SboltError};
use crate::keys::rsa2048;
use crate::pkcs7::build_detached_signed_data;

const AUTHVAR_ATTRIBUTE_SIZE: usize = 4;
const EFI_TIME_SIZE: usize = 16;
const WIN_CERT_UEFI_GUID_HEADER_SIZE: usize = 24;
const WIN_CERT_REVISION_2_0: u16 = 0x0200;
const WIN_CERT_TYPE_EFI_GUID: u16 = 0x0EF1;

/// Sign an EFI variable update with `EFI_VARIABLE_AUTHENTICATION_2`.
///
/// # Errors
///
/// Returns an error if timestamp generation or PKCS#7 construction fails.
pub fn sign(
    var_name: &str,
    vendor_guid: &Guid,
    attributes: VariableAttributes,
    content: &[u8],
    signer: &rsa2048::Signer,
    certificate: &Certificate,
) -> Result<Vec<u8>> {
    let timestamp = now()?;

    let descriptor = build_descriptor(var_name, vendor_guid, attributes, &timestamp, content);

    let pkcs7_der = build_detached_signed_data(&descriptor, signer, certificate)?;

    let win_cert = build_win_certificate(&pkcs7_der)?;

    let payload_capacity = checked_add(
        checked_add(
            AUTHVAR_ATTRIBUTE_SIZE,
            EFI_TIME_SIZE,
            "authvar payload header",
        )?,
        checked_add(win_cert.len(), content.len(), "authvar payload content")?,
        "authvar payload total",
    )?;
    let mut payload = Vec::with_capacity(payload_capacity);
    payload.extend_from_slice(&attributes.bits().to_le_bytes());
    payload.extend_from_slice(&time_to_bytes(&timestamp));
    payload.extend_from_slice(&win_cert);
    payload.extend_from_slice(content);

    Ok(payload)
}

/// Build the descriptor that gets signed per UEFI spec 8.2.2.
fn build_descriptor(
    var_name: &str,
    vendor_guid: &Guid,
    attributes: VariableAttributes,
    timestamp: &Time,
    content: &[u8],
) -> Vec<u8> {
    let mut desc = Vec::new();

    for code_unit in var_name.encode_utf16() {
        desc.extend_from_slice(&code_unit.to_le_bytes());
    }

    desc.extend_from_slice(&vendor_guid.to_bytes());
    desc.extend_from_slice(&attributes.bits().to_le_bytes());
    desc.extend_from_slice(&time_to_bytes(timestamp));
    desc.extend_from_slice(content);

    desc
}

/// Build `WIN_CERTIFICATE_UEFI_GUID` structure.
fn build_win_certificate(pkcs7_der: &[u8]) -> Result<Vec<u8>> {
    let total_size = checked_add(
        WIN_CERT_UEFI_GUID_HEADER_SIZE,
        pkcs7_der.len(),
        "WIN_CERTIFICATE_UEFI_GUID size",
    )?;
    let total_size_u32 = u32::try_from(total_size)
        .map_err(|_size_error| SboltError::EfiVar("WIN_CERTIFICATE size exceeds u32".into()))?;

    let mut result = Vec::with_capacity(total_size);

    result.extend_from_slice(&total_size_u32.to_le_bytes());
    result.extend_from_slice(&WIN_CERT_REVISION_2_0.to_le_bytes());
    result.extend_from_slice(&WIN_CERT_TYPE_EFI_GUID.to_le_bytes());
    result.extend_from_slice(&EFI_CERT_TYPE_PKCS7_GUID.to_bytes());
    result.extend_from_slice(pkcs7_der);

    Ok(result)
}

fn checked_add(lhs: usize, rhs: usize, context: &str) -> Result<usize> {
    lhs.checked_add(rhs)
        .ok_or_else(|| SboltError::EfiVar(format!("{context} overflow")))
}

#[cfg(test)]
mod tests {
    use uefi::guid;

    use super::*;
    use crate::keys::cert;
    use crate::keys::rsa2048;

    fn signer_and_cert() -> (rsa2048::Signer, Certificate) {
        let (pk_signer, pk_cert) = cert::generate_pk("Test PK").expect("generate PK cert");
        let (kek_signer, kek_cert) =
            cert::generate_kek("Test KEK", &pk_signer, &pk_cert).expect("generate KEK cert");
        let (db_signer, db_cert) =
            cert::generate_db("Test DB", &kek_signer, &kek_cert).expect("generate DB cert");
        (db_signer, db_cert)
    }

    #[test]
    fn build_descriptor_layout() {
        // ARRANGE
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

        // ACT
        let desc = build_descriptor(var_name, &vendor, attrs, &timestamp, content);

        // ASSERT
        let mut offset = 0;

        let name_utf16: Vec<u16> = var_name.encode_utf16().collect();
        let name_len_bytes = name_utf16.len() * 2;
        for ch in &name_utf16 {
            let got = u16::from_le_bytes([desc[offset], desc[offset + 1]]);
            assert_eq!(got, *ch, "UTF-16LE mismatch at offset {offset}");
            offset += 2;
        }

        assert_eq!(&desc[offset..offset + 16], &vendor.to_bytes());
        offset += 16;

        let got_attrs = u32::from_le_bytes(desc[offset..offset + 4].try_into().expect("4 bytes"));
        assert_eq!(got_attrs, attrs.bits());
        offset += 4;

        let time_bytes = super::time_to_bytes(&timestamp);
        assert_eq!(&desc[offset..offset + 16], &time_bytes);
        offset += 16;

        assert_eq!(&desc[offset..], content);

        assert_eq!(desc.len(), name_len_bytes + 16 + 4 + 16 + content.len());
    }

    #[test]
    fn build_win_certificate_layout() {
        // ARRANGE
        let fake_pkcs7 = b"fake-pkcs7-signature-data";

        // ACT
        let wc = build_win_certificate(fake_pkcs7).expect("build win certificate");

        // ASSERT
        let expected_total = 24 + fake_pkcs7.len();
        assert_eq!(wc.len(), expected_total);

        let total_size = u32::from_le_bytes(wc[0..4].try_into().expect("4 bytes"));
        assert_eq!(total_size as usize, expected_total);

        let revision = u16::from_le_bytes(wc[4..6].try_into().expect("2 bytes"));
        assert_eq!(revision, 0x0200);

        let cert_type = u16::from_le_bytes(wc[6..8].try_into().expect("2 bytes"));
        assert_eq!(cert_type, 0x0EF1);

        assert_eq!(&wc[8..24], &EFI_CERT_TYPE_PKCS7_GUID.to_bytes());

        assert_eq!(&wc[24..], fake_pkcs7);
    }

    #[test]
    fn sign_efi_variable_structural_correctness() {
        // ARRANGE
        let (signer, cert) = signer_and_cert();
        let var_name = "db";
        let vendor = guid!("d719b2cb-3d3a-4596-a3bc-dad00e67656f");
        let attrs = VariableAttributes::NON_VOLATILE
            | VariableAttributes::BOOTSERVICE_ACCESS
            | VariableAttributes::RUNTIME_ACCESS
            | VariableAttributes::TIME_BASED_AUTHENTICATED_WRITE_ACCESS;
        let content = b"test-siglist-data";

        // ACT
        let payload = sign(var_name, &vendor, attrs, content, &signer, &cert)
            .expect("sign_efi_variable should succeed");

        // ASSERT
        let got_attrs = u32::from_le_bytes(payload[0..4].try_into().expect("4 bytes"));
        assert_eq!(got_attrs, attrs.bits());

        let timestamp_bytes = &payload[4..20];
        assert!(
            timestamp_bytes.iter().any(|&b| b != 0),
            "timestamp should not be all zeros"
        );
        let year = u16::from_le_bytes([timestamp_bytes[0], timestamp_bytes[1]]);
        assert!(year >= 2024 && year <= 2100, "year {year} out of range");

        let win_cert_size =
            u32::from_le_bytes(payload[20..24].try_into().expect("4 bytes")) as usize;
        assert!(win_cert_size >= 24, "WIN_CERT must be at least 24 bytes");

        let revision = u16::from_le_bytes(payload[24..26].try_into().expect("2 bytes"));
        assert_eq!(revision, 0x0200);

        let cert_type = u16::from_le_bytes(payload[26..28].try_into().expect("2 bytes"));
        assert_eq!(cert_type, 0x0EF1);

        assert_eq!(&payload[28..44], &EFI_CERT_TYPE_PKCS7_GUID.to_bytes());

        assert_eq!(
            &payload[payload.len() - content.len()..],
            content,
            "payload must end with the content bytes"
        );

        assert_eq!(payload.len(), 4 + 16 + win_cert_size + content.len());
    }

    #[test]
    fn checked_add_rejects_overflow() {
        // ACT
        let result = checked_add(usize::MAX, 1, "authvar");

        // ASSERT
        assert!(result.is_err());
    }
}
