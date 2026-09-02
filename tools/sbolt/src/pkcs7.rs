//! PKCS#7/CMS `SignedData` construction.

use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::{CmsVersion, ContentInfo};
use cms::signed_data::{
    CertificateSet, EncapsulatedContentInfo, SignatureValue, SignedAttributes, SignedData,
    SignerIdentifier, SignerInfo, SignerInfos,
};
use const_oid::ObjectIdentifier;
use const_oid::db::rfc5912::ID_SHA_256;
use der::asn1::{Any, OctetString, SetOfVec, UtcTime};
use der::{Decode as _, Encode as _};
use ring::digest::{Context, SHA256};
use x509_cert::Certificate;
use x509_cert::attr::{Attribute, AttributeValue};

use crate::error::{Result, SboltError};
use crate::keys::rsa2048;

const ID_CONTENT_TYPE: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.3");
const ID_MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
const ID_SIGNING_TIME: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.5");
const RSA_ENCRYPTION: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const ID_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
const ID_SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");

pub(crate) const DER_SHORT_FORM_LENGTH_LIMIT: usize = 0x80;
pub(crate) const DER_ONE_BYTE_LENGTH_LIMIT: usize = 0x100;
pub(crate) const DER_TWO_BYTE_LENGTH_LIMIT: usize = 0x1_0000;

/// Build PKCS#7 `SignedData` for Authenticode PE signing.
pub(crate) fn build_authenticode_signed_data(
    content_type: ObjectIdentifier,
    content: &[u8],
    signature: &[u8],
    certificate: &Certificate,
    signed_attrs: Option<SignedAttributes>,
) -> Result<Vec<u8>> {
    let signed_data_der = build_signed_data_with_cms(
        content_type,
        Some(content),
        signature,
        certificate,
        signed_attrs,
    )?;

    wrap_signed_data_content_info(&signed_data_der)
}

/// Compute the `WIN_CERTIFICATE` size (including 8-byte alignment).
pub(crate) fn compute_authenticode_size(
    content_type: ObjectIdentifier,
    content: &[u8],
    certificate: &Certificate,
) -> Result<usize> {
    let mut ctx = Context::new(&SHA256);
    ctx.update(content);
    let digest = ctx.finish();
    let digest_bytes = parse_sha256_digest(digest.as_ref())?;
    let signed_attrs = build_signed_attributes(content_type, &digest_bytes)?;
    let zero_sig = [0_u8; 256];
    let signed_data_der = build_signed_data_with_cms(
        content_type,
        Some(content),
        &zero_sig,
        certificate,
        Some(signed_attrs),
    )?;
    let content_info_der = wrap_signed_data_content_info(&signed_data_der)?;
    let aligned_size = content_info_der
        .len()
        .checked_add(8)
        .ok_or_else(|| SboltError::Signing("WIN_CERTIFICATE size overflow".into()))?;
    let adjusted = aligned_size
        .checked_add(7)
        .ok_or_else(|| SboltError::Signing("alignment overflow".into()))?;

    Ok(adjusted & !7)
}

/// Build PKCS#7 `SignedData` for EFI authenticated variable signing (detached).
pub(crate) fn build_detached_signed_data(
    data: &[u8],
    signer: &rsa2048::Signer,
    certificate: &Certificate,
) -> Result<Vec<u8>> {
    let mut ctx = Context::new(&SHA256);
    ctx.update(data);
    let digest = ctx.finish();
    let digest_bytes = parse_sha256_digest(digest.as_ref())?;

    let signed_attrs = build_signed_attributes(ID_DATA, &digest_bytes)?;

    let attrs_for_signing = signed_attrs
        .to_der()
        .map_err(|e| SboltError::Signing(format!("encode signed attrs: {e}")))?;

    let sig = signer.sign_pkcs1v15_sha256(&attrs_for_signing)?;

    let signed_data_der =
        build_signed_data_with_cms(ID_DATA, None, &sig, certificate, Some(signed_attrs))?;

    wrap_signed_data_content_info(&signed_data_der)
}

/// Build `SignedData` with optional content and signed attributes.
fn build_signed_data_with_cms(
    content_type: ObjectIdentifier,
    content: Option<&[u8]>,
    signature: &[u8],
    certificate: &Certificate,
    signed_attrs: Option<SignedAttributes>,
) -> Result<Vec<u8>> {
    let digest_alg = spki::AlgorithmIdentifierOwned {
        oid: ID_SHA_256,
        parameters: Some(Any::null()),
    };
    let mut digest_algs = SetOfVec::new();
    digest_algs
        .insert(digest_alg.clone())
        .map_err(|e| SboltError::Signing(format!("digest alg set: {e}")))?;

    let sequence_tag = 0x30;

    let econtent = match content {
        Some(inner_fields) => {
            let mut seq = Vec::new();
            seq.push(sequence_tag);
            encode_der_length(&mut seq, inner_fields.len())?;
            seq.extend_from_slice(inner_fields);

            Some(
                Any::from_der(&seq)
                    .map_err(|e| SboltError::Signing(format!("econtent from der: {e}")))?,
            )
        }
        None => None,
    };

    let encap_content_info = EncapsulatedContentInfo {
        econtent_type: content_type,
        econtent,
    };

    let cert_choice = CertificateChoices::Certificate(certificate.clone());
    let mut certs_vec = SetOfVec::new();
    certs_vec
        .insert(cert_choice)
        .map_err(|e| SboltError::Signing(format!("cert set: {e}")))?;
    let certificates = Some(CertificateSet(certs_vec));

    let signer_info = build_signer_info_cms(signature, certificate, signed_attrs)?;
    let mut signer_infos_vec = SetOfVec::new();
    signer_infos_vec
        .insert(signer_info)
        .map_err(|e| SboltError::Signing(format!("signer info set: {e}")))?;

    let signed_data = SignedData {
        version: CmsVersion::V1,
        digest_algorithms: digest_algs,
        encap_content_info,
        certificates,
        crls: None,
        signer_infos: SignerInfos(signer_infos_vec),
    };

    signed_data
        .to_der()
        .map_err(|e| SboltError::Signing(format!("signed data encode: {e}")))
}

/// Build `SignerInfo` using CMS types.
fn build_signer_info_cms(
    signature: &[u8],
    certificate: &Certificate,
    signed_attrs: Option<SignedAttributes>,
) -> Result<SignerInfo> {
    let issuer = certificate.tbs_certificate().issuer().clone();
    let serial = certificate.tbs_certificate().serial_number().clone();
    let sid = SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
        issuer,
        serial_number: serial,
    });

    let digest_alg = spki::AlgorithmIdentifierOwned {
        oid: ID_SHA_256,
        parameters: Some(Any::null()),
    };

    let sig_alg = spki::AlgorithmIdentifierOwned {
        oid: RSA_ENCRYPTION,
        parameters: Some(Any::null()),
    };

    let sig_value = SignatureValue::new(signature)
        .map_err(|e| SboltError::Signing(format!("sig value: {e}")))?;

    Ok(SignerInfo {
        version: CmsVersion::V1,
        sid,
        digest_alg,
        signed_attrs,
        signature_algorithm: sig_alg,
        signature: sig_value,
        unsigned_attrs: None,
    })
}

/// Build signed attributes for PKCS#7 `SignedData` following RFC 5652.
pub(crate) fn build_signed_attributes(
    content_type: ObjectIdentifier,
    message_digest: &[u8; 32],
) -> Result<SignedAttributes> {
    let content_type_value = content_type
        .to_der()
        .map_err(|e| SboltError::Signing(format!("encode content type OID: {e}")))?;
    let content_type_attr_value = AttributeValue::from(
        Any::from_der(&content_type_value)
            .map_err(|e| SboltError::Signing(format!("content type any: {e}")))?,
    );
    let mut content_type_values = SetOfVec::new();
    content_type_values
        .insert(content_type_attr_value)
        .map_err(|e| SboltError::Signing(format!("content type set: {e}")))?;
    let content_type_attr = Attribute {
        oid: ID_CONTENT_TYPE,
        values: content_type_values,
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let utc_time = UtcTime::from_unix_duration(now)
        .map_err(|e| SboltError::Signing(format!("utc time: {e}")))?;
    let utc_time_der = utc_time
        .to_der()
        .map_err(|e| SboltError::Signing(format!("utc time der: {e}")))?;
    let signing_time_attr_value = AttributeValue::from(
        Any::from_der(&utc_time_der)
            .map_err(|e| SboltError::Signing(format!("signing time any: {e}")))?,
    );
    let mut signing_time_values = SetOfVec::new();
    signing_time_values
        .insert(signing_time_attr_value)
        .map_err(|e| SboltError::Signing(format!("signing time set: {e}")))?;
    let signing_time_attr = Attribute {
        oid: ID_SIGNING_TIME,
        values: signing_time_values,
    };

    let digest_octet = OctetString::new(message_digest.to_vec())
        .map_err(|e| SboltError::Signing(format!("digest octet string: {e}")))?;
    let digest_der = digest_octet
        .to_der()
        .map_err(|e| SboltError::Signing(format!("digest der: {e}")))?;
    let digest_attr_value = AttributeValue::from(
        Any::from_der(&digest_der).map_err(|e| SboltError::Signing(format!("digest any: {e}")))?,
    );
    let mut digest_values = SetOfVec::new();
    digest_values
        .insert(digest_attr_value)
        .map_err(|e| SboltError::Signing(format!("digest set: {e}")))?;
    let message_digest_attr = Attribute {
        oid: ID_MESSAGE_DIGEST,
        values: digest_values,
    };

    let mut attrs: SignedAttributes = SetOfVec::new();
    attrs
        .insert(content_type_attr)
        .map_err(|e| SboltError::Signing(format!("insert content type attr: {e}")))?;
    attrs
        .insert(signing_time_attr)
        .map_err(|e| SboltError::Signing(format!("insert signing time attr: {e}")))?;
    attrs
        .insert(message_digest_attr)
        .map_err(|e| SboltError::Signing(format!("insert message digest attr: {e}")))?;

    Ok(attrs)
}

/// Encode ASN.1 DER length bytes.
pub(crate) fn encode_der_length(buf: &mut Vec<u8>, len: usize) -> Result<()> {
    if len < DER_SHORT_FORM_LENGTH_LIMIT {
        push_u8(buf, len & 0xff)?;
    } else if len < DER_ONE_BYTE_LENGTH_LIMIT {
        buf.push(0x81);
        push_u8(buf, len & 0xff)?;
    } else if len < DER_TWO_BYTE_LENGTH_LIMIT {
        buf.push(0x82);
        push_u8(buf, len >> 8)?;
        push_u8(buf, len & 0xff)?;
    } else {
        buf.push(0x83);
        push_u8(buf, len >> 16)?;
        push_u8(buf, (len >> 8) & 0xff)?;
        push_u8(buf, len & 0xff)?;
    }

    Ok(())
}

fn parse_sha256_digest(digest: &[u8]) -> Result<[u8; 32]> {
    digest.try_into().map_err(|_digest_length_error| {
        SboltError::Signing("SHA-256 digest is not 32 bytes".to_owned())
    })
}

pub(crate) fn push_u8(buf: &mut Vec<u8>, value: usize) -> Result<()> {
    let byte = u8::try_from(value).map_err(|_conversion_error| {
        SboltError::Signing(format!("DER length byte out of range: {value}"))
    })?;
    buf.push(byte);

    Ok(())
}

fn wrap_signed_data_content_info(signed_data_der: &[u8]) -> Result<Vec<u8>> {
    let signed_data_any = Any::from_der(signed_data_der)
        .map_err(|e| SboltError::Signing(format!("signed data as Any: {e}")))?;

    let content_info = ContentInfo {
        content_type: ID_SIGNED_DATA,
        content: signed_data_any,
    };

    content_info
        .to_der()
        .map_err(|e| SboltError::Signing(format!("content info encode: {e}")))
}

#[cfg(test)]
mod tests {
    use cms::content_info::ContentInfo;
    use cms::signed_data::SignedData;
    use der::asn1::OctetString;

    use super::*;
    use crate::keys::cert;

    fn signer_and_cert() -> Result<(rsa2048::Signer, Certificate)> {
        let (pk_signer, pk_cert) = cert::generate_pk("PK")?;
        let (kek_signer, kek_cert) = cert::generate_kek("KEK", &pk_signer, &pk_cert)?;
        cert::generate_db("DB", &kek_signer, &kek_cert)
    }

    #[test]
    fn build_authenticode_signed_data_returns_signed_data_content_info() {
        // ARRANGE
        let (signer, cert) = signer_and_cert().expect("generate signer and certificate");
        let content = [0x30_u8, 0x00];
        let mut ctx = Context::new(&SHA256);
        ctx.update(&content);
        let digest = ctx.finish();
        let digest_bytes = parse_sha256_digest(digest.as_ref()).expect("parse digest");
        let signed_attrs =
            build_signed_attributes(ID_DATA, &digest_bytes).expect("build signed attrs");
        let attrs_der = signed_attrs.to_der().expect("encode signed attrs");
        let sig = signer.sign_pkcs1v15_sha256(&attrs_der).expect("sign attrs");

        // ACT
        let der =
            build_authenticode_signed_data(ID_DATA, &content, &sig, &cert, Some(signed_attrs))
                .expect("build Authenticode SignedData");
        let content_info = ContentInfo::from_der(&der).expect("decode ContentInfo");

        // ASSERT
        assert_eq!(content_info.content_type, ID_SIGNED_DATA);
    }

    #[test]
    fn build_authenticode_signed_data_embeds_content_and_signer_info() {
        // ARRANGE
        let (signer, cert) = signer_and_cert().expect("generate signer and certificate");
        let content = [0x30_u8, 0x00];
        let mut ctx = Context::new(&SHA256);
        ctx.update(&content);
        let digest = ctx.finish();
        let digest_bytes = parse_sha256_digest(digest.as_ref()).expect("parse digest");
        let signed_attrs =
            build_signed_attributes(ID_DATA, &digest_bytes).expect("build signed attrs");
        let attrs_der = signed_attrs.to_der().expect("encode signed attrs");
        let sig = signer.sign_pkcs1v15_sha256(&attrs_der).expect("sign attrs");

        // ACT
        let der =
            build_authenticode_signed_data(ID_DATA, &content, &sig, &cert, Some(signed_attrs))
                .expect("build Authenticode SignedData");
        let content_info = ContentInfo::from_der(&der).expect("decode ContentInfo");
        let signed_data = content_info
            .content
            .decode_as::<SignedData>()
            .expect("decode SignedData");

        // ASSERT
        assert!(signed_data.encap_content_info.econtent.is_some());
        assert_eq!(signed_data.signer_infos.0.len(), 1);
    }

    #[test]
    fn build_detached_signed_data_omits_econtent() {
        // ARRANGE
        let (signer, cert) = signer_and_cert().expect("generate signer and certificate");

        // ACT
        let der = build_detached_signed_data(b"payload", &signer, &cert).expect("build SignedData");
        let content_info = ContentInfo::from_der(&der).expect("decode ContentInfo");
        assert_eq!(content_info.content_type, ID_SIGNED_DATA);
        let signed_data = content_info
            .content
            .decode_as::<SignedData>()
            .expect("decode SignedData");

        // ASSERT
        assert!(signed_data.encap_content_info.econtent.is_none());
        assert_eq!(signed_data.signer_infos.0.len(), 1);
    }

    #[test]
    fn encode_der_length_supports_multiple_length_forms() {
        // ARRANGE
        let mut short = Vec::new();
        let mut medium = Vec::new();
        let mut long = Vec::new();

        // ACT
        encode_der_length(&mut short, 0x7f).expect("encode short length");
        encode_der_length(&mut medium, 0x80).expect("encode medium length");
        encode_der_length(&mut long, 0x1234).expect("encode long length");

        // ASSERT
        assert_eq!(short, vec![0x7f]);
        assert_eq!(medium, vec![0x81, 0x80]);
        assert_eq!(long, vec![0x82, 0x12, 0x34]);
    }

    #[test]
    fn parse_sha256_digest_rejects_wrong_length() {
        // ACT
        let result = parse_sha256_digest(&[0_u8; 31]);

        // ASSERT
        result.expect_err("wrong digest length should fail");
    }

    #[test]
    fn parse_sha256_digest_accepts_exact_length() {
        // ARRANGE
        let digest = [0x5a_u8; 32];

        // ACT
        let parsed = parse_sha256_digest(&digest).expect("parse digest");

        // ASSERT
        assert_eq!(parsed, digest);
    }

    #[test]
    fn encode_der_length_supports_three_byte_lengths() {
        // ARRANGE
        let mut encoded = Vec::new();

        // ACT
        encode_der_length(&mut encoded, 0x01_0203).expect("encode length");

        // ASSERT
        assert_eq!(encoded, vec![0x83, 0x01, 0x02, 0x03]);
    }

    #[test]
    fn wrap_signed_data_content_info_rejects_invalid_der() {
        // ACT
        let result = wrap_signed_data_content_info(&[0_u8; 1]);

        // ASSERT
        result.expect_err("invalid DER should fail");
    }

    #[test]
    fn encode_der_length_rejects_four_byte_lengths() {
        // ARRANGE
        let mut encoded = Vec::new();

        // ACT
        let result = encode_der_length(&mut encoded, 0x01_000000);

        // ASSERT
        result.expect_err("four-byte length should fail");
    }

    #[test]
    fn push_u8_rejects_values_larger_than_byte() {
        // ARRANGE
        let mut encoded = Vec::new();

        // ACT
        let result = push_u8(&mut encoded, 0x100);

        // ASSERT
        result.expect_err("large byte value should fail");
        assert!(encoded.is_empty());
    }

    #[test]
    fn build_signed_attributes_contains_expected_values() {
        // ARRANGE
        let digest = [0xA5_u8; 32];

        // ACT
        let attrs = build_signed_attributes(ID_DATA, &digest).expect("build signed attributes");

        // ASSERT
        assert_eq!(attrs.len(), 3);

        let digest_attr = attrs
            .iter()
            .find(|attr| attr.oid == ID_MESSAGE_DIGEST)
            .expect("messageDigest attribute");
        let digest_value = digest_attr.values.iter().next().expect("digest value");
        let digest_octets = digest_value
            .decode_as::<OctetString>()
            .expect("decode messageDigest");
        assert_eq!(digest_octets.as_bytes(), digest);
    }

    #[test]
    fn build_signer_info_uses_certificate_identity() {
        // ARRANGE
        let (_signer, cert) = signer_and_cert().expect("generate signer and certificate");
        let signature = [0x11_u8; 16];
        let attrs =
            build_signed_attributes(ID_DATA, &[0x22_u8; 32]).expect("build signed attributes");

        // ACT
        let signer_info =
            build_signer_info_cms(&signature, &cert, Some(attrs)).expect("build signer info");

        // ASSERT
        match signer_info.sid {
            SignerIdentifier::IssuerAndSerialNumber(issuer_and_serial) => {
                assert_eq!(
                    issuer_and_serial.issuer,
                    cert.tbs_certificate().issuer().clone()
                );
                assert_eq!(
                    issuer_and_serial.serial_number,
                    cert.tbs_certificate().serial_number().clone()
                );
            }
            SignerIdentifier::SubjectKeyIdentifier(_) => panic!("expected IssuerAndSerialNumber"),
        }
    }

    #[test]
    fn build_signed_data_with_content_embeds_econtent() {
        // ARRANGE
        let (_signer, cert) = signer_and_cert().expect("generate signer and certificate");
        let content = [0x30_u8, 0x00];
        let attrs =
            build_signed_attributes(ID_DATA, &[0x33_u8; 32]).expect("build signed attributes");

        // ACT
        let signed_data_der =
            build_signed_data_with_cms(ID_DATA, Some(&content), &[0x44_u8; 8], &cert, Some(attrs))
                .expect("build SignedData");
        let signed_data = SignedData::from_der(&signed_data_der).expect("decode SignedData");

        // ASSERT
        assert!(signed_data.encap_content_info.econtent.is_some());
    }

    #[test]
    fn build_signed_data_without_attrs_keeps_signer_info_unsigned() {
        // ARRANGE
        let (_signer, cert) = signer_and_cert().expect("generate signer and certificate");

        // ACT
        let signed_data_der = build_signed_data_with_cms(ID_DATA, None, &[0x55_u8; 8], &cert, None)
            .expect("build SignedData");
        let signed_data = SignedData::from_der(&signed_data_der).expect("decode SignedData");
        let signer_info = signed_data
            .signer_infos
            .0
            .iter()
            .next()
            .expect("signer info");

        // ASSERT
        assert!(signed_data.encap_content_info.econtent.is_none());
        assert!(signer_info.signed_attrs.is_none());
    }
}
