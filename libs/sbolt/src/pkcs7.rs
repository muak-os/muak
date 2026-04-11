//! PKCS#7/CMS SignedData construction

use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::{CmsVersion, ContentInfo};
use cms::signed_data::{
    CertificateSet, EncapsulatedContentInfo, SignedAttributes, SignedData, SignerIdentifier,
    SignerInfo, SignerInfos,
};
use const_oid::ObjectIdentifier;
use const_oid::db::rfc5912::ID_SHA_256;
use der::asn1::{OctetString, SetOfVec, UtcTime};
use der::{Decode, Encode};
use ring::digest::{Context, SHA256};
use x509_cert::Certificate;
use x509_cert::attr::{Attribute, AttributeValue};

use crate::keys::Rsa2048Signer;
use crate::{Error, Result};

const ID_CONTENT_TYPE: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.3");
const ID_MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
const ID_SIGNING_TIME: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.5");
const RSA_ENCRYPTION: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const ID_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
const ID_SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");

/// Build PKCS#7 SignedData for Authenticode PE signing.
///
/// The `content` parameter must be the **inner fields** of
/// `SpcIndirectDataContent` (the concatenated child DER elements WITHOUT
/// the outer SEQUENCE wrapper).
pub fn build_authenticode_signed_data(
    content_type: ObjectIdentifier,
    content: &[u8],
    _hash: &[u8; 32],
    signer: &Rsa2048Signer,
    certificate: &Certificate,
) -> Result<Vec<u8>> {
    let mut ctx = Context::new(&SHA256);
    ctx.update(content);
    let digest = ctx.finish();
    let digest_bytes: [u8; 32] = digest
        .as_ref()
        .try_into()
        .map_err(|_| Error::Signing("SHA-256 digest is not 32 bytes".to_string()))?;

    let signed_attrs = build_signed_attributes(content_type, &digest_bytes)?;

    let attrs_for_signing = signed_attrs
        .to_der()
        .map_err(|e| Error::Signing(format!("encode signed attrs: {e}")))?;

    let sig = signer.sign(&attrs_for_signing)?;

    let signed_data_der = build_signed_data_with_cms(
        content_type,
        Some(content),
        &sig,
        certificate,
        Some(signed_attrs),
    )?;

    let signed_data_any = der::asn1::Any::from_der(&signed_data_der)
        .map_err(|e| Error::Signing(format!("signed data as Any: {e}")))?;

    let content_info = ContentInfo {
        content_type: ID_SIGNED_DATA,
        content: signed_data_any,
    };

    content_info
        .to_der()
        .map_err(|e| Error::Signing(format!("content info encode: {e}")))
}

/// Build PKCS#7 SignedData for EFI authenticated variable signing (detached)
pub fn build_detached_signed_data(
    data: &[u8],
    signer: &Rsa2048Signer,
    certificate: &Certificate,
) -> Result<Vec<u8>> {
    let mut ctx = Context::new(&SHA256);
    ctx.update(data);
    let digest = ctx.finish();
    let digest_bytes: [u8; 32] = digest
        .as_ref()
        .try_into()
        .map_err(|_| Error::Signing("SHA-256 digest is not 32 bytes".to_string()))?;

    let signed_attrs = build_signed_attributes(ID_DATA, &digest_bytes)?;

    let attrs_for_signing = signed_attrs
        .to_der()
        .map_err(|e| Error::Signing(format!("encode signed attrs: {e}")))?;

    let sig = signer.sign(&attrs_for_signing)?;

    build_signed_data_with_cms(ID_DATA, None, &sig, certificate, Some(signed_attrs))
}

/// Build SignedData structure using cms types, with optional content and signed attributes
fn build_signed_data_with_cms(
    content_type: ObjectIdentifier,
    content: Option<&[u8]>,
    signature: &[u8],
    certificate: &Certificate,
    signed_attrs: Option<SignedAttributes>,
) -> Result<Vec<u8>> {
    let digest_alg = spki::AlgorithmIdentifierOwned {
        oid: ID_SHA_256,
        parameters: Some(der::asn1::Any::null()),
    };
    let mut digest_algs = SetOfVec::new();
    digest_algs
        .insert(digest_alg.clone())
        .map_err(|e| Error::Signing(format!("digest alg set: {e}")))?;

    let sequence_tag = 0x30;

    let econtent = match content {
        Some(inner_fields) => {
            let mut seq = Vec::new();
            seq.push(sequence_tag);
            encode_der_length(&mut seq, inner_fields.len());
            seq.extend_from_slice(inner_fields);

            Some(
                der::asn1::Any::from_der(&seq)
                    .map_err(|e| Error::Signing(format!("econtent from der: {e}")))?,
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
        .map_err(|e| Error::Signing(format!("cert set: {e}")))?;
    let certificates = Some(CertificateSet(certs_vec));

    let signer_info = build_signer_info_cms(signature, certificate, signed_attrs)?;
    let mut signer_infos_vec = SetOfVec::new();
    signer_infos_vec
        .insert(signer_info)
        .map_err(|e| Error::Signing(format!("signer info set: {e}")))?;

    let signed_data = SignedData {
        version: CmsVersion::V1,
        digest_algorithms: digest_algs,
        encap_content_info,
        certificates,
        crls: None,
        signer_infos: SignerInfos(signer_infos_vec),
    };

    // Return just the SignedData DER, NOT wrapped in ContentInfo.
    // The caller handles ContentInfo wrapping.
    signed_data
        .to_der()
        .map_err(|e| Error::Signing(format!("signed data encode: {e}")))
}

/// Build SignerInfo using cms types
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
        parameters: Some(der::asn1::Any::null()),
    };

    let sig_alg = spki::AlgorithmIdentifierOwned {
        oid: RSA_ENCRYPTION,
        parameters: Some(der::asn1::Any::null()),
    };

    let sig_value = cms::signed_data::SignatureValue::new(signature)
        .map_err(|e| Error::Signing(format!("sig value: {e}")))?;

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

/// Build signed attributes for PKCS#7 SignedData following RFC 5652
fn build_signed_attributes(
    content_type: ObjectIdentifier,
    message_digest: &[u8; 32],
) -> Result<SignedAttributes> {
    let content_type_value = content_type
        .to_der()
        .map_err(|e| Error::Signing(format!("encode content type OID: {e}")))?;
    let content_type_attr_value = AttributeValue::from(
        der::asn1::Any::from_der(&content_type_value)
            .map_err(|e| Error::Signing(format!("content type any: {e}")))?,
    );
    let mut content_type_values = SetOfVec::new();
    content_type_values
        .insert(content_type_attr_value)
        .map_err(|e| Error::Signing(format!("content type set: {e}")))?;
    let content_type_attr = Attribute {
        oid: ID_CONTENT_TYPE,
        values: content_type_values,
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let utc_time =
        UtcTime::from_unix_duration(now).map_err(|e| Error::Signing(format!("utc time: {e}")))?;
    let utc_time_der = utc_time
        .to_der()
        .map_err(|e| Error::Signing(format!("utc time der: {e}")))?;
    let signing_time_attr_value = AttributeValue::from(
        der::asn1::Any::from_der(&utc_time_der)
            .map_err(|e| Error::Signing(format!("signing time any: {e}")))?,
    );
    let mut signing_time_values = SetOfVec::new();
    signing_time_values
        .insert(signing_time_attr_value)
        .map_err(|e| Error::Signing(format!("signing time set: {e}")))?;
    let signing_time_attr = Attribute {
        oid: ID_SIGNING_TIME,
        values: signing_time_values,
    };

    let digest_octet = OctetString::new(message_digest.to_vec())
        .map_err(|e| Error::Signing(format!("digest octet string: {e}")))?;
    let digest_der = digest_octet
        .to_der()
        .map_err(|e| Error::Signing(format!("digest der: {e}")))?;
    let digest_attr_value = AttributeValue::from(
        der::asn1::Any::from_der(&digest_der)
            .map_err(|e| Error::Signing(format!("digest any: {e}")))?,
    );
    let mut digest_values = SetOfVec::new();
    digest_values
        .insert(digest_attr_value)
        .map_err(|e| Error::Signing(format!("digest set: {e}")))?;
    let message_digest_attr = Attribute {
        oid: ID_MESSAGE_DIGEST,
        values: digest_values,
    };

    let mut attrs: SignedAttributes = SetOfVec::new();
    attrs
        .insert(content_type_attr)
        .map_err(|e| Error::Signing(format!("insert content type attr: {e}")))?;
    attrs
        .insert(signing_time_attr)
        .map_err(|e| Error::Signing(format!("insert signing time attr: {e}")))?;
    attrs
        .insert(message_digest_attr)
        .map_err(|e| Error::Signing(format!("insert message digest attr: {e}")))?;

    Ok(attrs)
}

/// Encode ASN.1 DER length bytes
fn encode_der_length(buf: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        buf.push(len as u8);
    } else if len < 0x100 {
        buf.push(0x81);
        buf.push(len as u8);
    } else if len < 0x10000 {
        buf.push(0x82);
        buf.push((len >> 8) as u8);
        buf.push(len as u8);
    } else {
        buf.push(0x83);
        buf.push((len >> 16) as u8);
        buf.push((len >> 8) as u8);
        buf.push(len as u8);
    }
}
