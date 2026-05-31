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
    use uefi::runtime::{Daylight, Time, TimeParams};

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

        let timestamp = Time::new(TimeParams {
            year: 2025,
            month: 1,
            day: 15,
            hour: 12,
            minute: 0,
            second: 0,
            nanosecond: 0,
            time_zone: Some(0),
            daylight: Daylight::empty(),
        })
        .expect("valid time");

        let content = b"test-content";

        // ACT
        let desc = build_descriptor(var_name, &vendor, attrs, &timestamp, content);

        // ASSERT
        let mut offset = 0_usize;

        let name_utf16: Vec<u16> = var_name.encode_utf16().collect();
        let name_len_bytes = name_utf16.len().checked_mul(2).expect("name byte length");
        for character in &name_utf16 {
            let end = offset.checked_add(2).expect("UTF-16 end offset");
            let got = u16::from_le_bytes(
                desc.get(offset..end)
                    .expect("UTF-16 bytes")
                    .try_into()
                    .expect("2 bytes"),
            );
            assert_eq!(got, *character, "UTF-16LE mismatch at offset {offset}");
            offset = end;
        }

        let vendor_end = offset.checked_add(16).expect("vendor end offset");
        assert_eq!(
            desc.get(offset..vendor_end).expect("vendor bytes"),
            &vendor.to_bytes()
        );
        offset = vendor_end;

        let attrs_end = offset.checked_add(4).expect("attrs end offset");
        let got_attrs = u32::from_le_bytes(
            desc.get(offset..attrs_end)
                .expect("attribute bytes")
                .try_into()
                .expect("4 bytes"),
        );
        assert_eq!(got_attrs, attrs.bits());
        offset = attrs_end;

        let time_bytes = super::time_to_bytes(&timestamp);
        let time_end = offset.checked_add(16).expect("time end offset");
        assert_eq!(desc.get(offset..time_end).expect("time bytes"), &time_bytes);
        offset = time_end;

        assert_eq!(desc.get(offset..).expect("content bytes"), content);

        let expected_len = [name_len_bytes, 16, 4, 16, content.len()]
            .into_iter()
            .try_fold(0_usize, usize::checked_add)
            .expect("descriptor length");
        assert_eq!(desc.len(), expected_len);
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

        let total_size = u32::from_le_bytes(
            wc.get(0..4)
                .expect("total size bytes")
                .try_into()
                .expect("4 bytes"),
        );
        assert_eq!(
            usize::try_from(total_size).expect("total size fits usize"),
            expected_total
        );

        let revision = u16::from_le_bytes(
            wc.get(4..6)
                .expect("revision bytes")
                .try_into()
                .expect("2 bytes"),
        );
        assert_eq!(revision, 0x0200);

        let cert_type = u16::from_le_bytes(
            wc.get(6..8)
                .expect("certificate type bytes")
                .try_into()
                .expect("2 bytes"),
        );
        assert_eq!(cert_type, 0x0EF1);

        assert_eq!(
            wc.get(8..24).expect("certificate GUID bytes"),
            &EFI_CERT_TYPE_PKCS7_GUID.to_bytes()
        );

        assert_eq!(wc.get(24..).expect("PKCS#7 bytes"), fake_pkcs7);
    }

    #[test]
    fn build_win_certificate_accepts_empty_pkcs7() {
        // ARRANGE
        let pkcs7 = [];

        // ACT
        let certificate = build_win_certificate(&pkcs7).expect("build WIN_CERTIFICATE");

        // ASSERT
        assert_eq!(certificate.len(), WIN_CERT_UEFI_GUID_HEADER_SIZE);
        assert_eq!(
            u32::from_le_bytes(
                certificate
                    .get(0..4)
                    .expect("length bytes")
                    .try_into()
                    .expect("4 bytes")
            ),
            u32::try_from(WIN_CERT_UEFI_GUID_HEADER_SIZE).expect("header size fits u32")
        );
    }

    #[test]
    fn build_descriptor_handles_empty_name_and_content() {
        // ARRANGE
        let vendor = guid!("d719b2cb-3d3a-4596-a3bc-dad00e67656f");
        let attrs = VariableAttributes::NON_VOLATILE;
        let timestamp = Time::new(TimeParams {
            year: 2025,
            month: 1,
            day: 15,
            hour: 12,
            minute: 0,
            second: 0,
            nanosecond: 0,
            time_zone: Some(0),
            daylight: Daylight::empty(),
        })
        .expect("valid time");

        // ACT
        let descriptor = build_descriptor("", &vendor, attrs, &timestamp, &[]);

        // ASSERT
        assert_eq!(descriptor.len(), 16 + 4 + 16);
        assert_eq!(
            descriptor.get(0..16).expect("vendor bytes"),
            &vendor.to_bytes()
        );
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
        let got_attrs = u32::from_le_bytes(
            payload
                .get(0..4)
                .expect("attribute bytes")
                .try_into()
                .expect("4 bytes"),
        );
        assert_eq!(got_attrs, attrs.bits());

        let timestamp_bytes = payload.get(4..20).expect("timestamp bytes");
        assert!(
            timestamp_bytes.iter().any(|&byte| byte != 0),
            "timestamp should not be all zeros"
        );
        let year = u16::from_le_bytes(
            timestamp_bytes
                .get(0..2)
                .expect("year bytes")
                .try_into()
                .expect("2 bytes"),
        );
        assert!((2024..=2100).contains(&year), "year {year} out of range");

        let win_cert_size = usize::try_from(u32::from_le_bytes(
            payload
                .get(20..24)
                .expect("certificate size bytes")
                .try_into()
                .expect("4 bytes"),
        ))
        .expect("cert size fits usize");
        assert!(win_cert_size >= 24, "WIN_CERT must be at least 24 bytes");

        let revision = u16::from_le_bytes(
            payload
                .get(24..26)
                .expect("revision bytes")
                .try_into()
                .expect("2 bytes"),
        );
        assert_eq!(revision, 0x0200);

        let cert_type = u16::from_le_bytes(
            payload
                .get(26..28)
                .expect("certificate type bytes")
                .try_into()
                .expect("2 bytes"),
        );
        assert_eq!(cert_type, 0x0EF1);

        assert_eq!(
            payload.get(28..44).expect("certificate GUID bytes"),
            &EFI_CERT_TYPE_PKCS7_GUID.to_bytes()
        );

        let content_offset = payload
            .len()
            .checked_sub(content.len())
            .expect("payload contains content");
        assert_eq!(
            payload.get(content_offset..).expect("content bytes"),
            content,
            "payload must end with the content bytes"
        );

        let expected_len = [4, 16, win_cert_size, content.len()]
            .into_iter()
            .try_fold(0_usize, usize::checked_add)
            .expect("payload length");
        assert_eq!(payload.len(), expected_len);
    }

    #[test]
    fn checked_add_rejects_overflow() {
        // ACT
        let result = checked_add(usize::MAX, 1, "authvar");

        // ASSERT
        result.expect_err("overflow should fail");
    }
}
