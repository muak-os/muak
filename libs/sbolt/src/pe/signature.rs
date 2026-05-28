//! PE signature embedding and PKCS#7 `SignedData` creation.

use core::mem::offset_of;
use core::mem::size_of;

use const_oid::ObjectIdentifier;
use der::Encode as _;
use der::asn1::{Any, OctetString};
use object::pe::{
    IMAGE_DIRECTORY_ENTRY_SECURITY, ImageDataDirectory, ImageFileHeader, ImageOptionalHeader64,
};
use object::read::pe::PeFile64;
use spki::AlgorithmIdentifierOwned;
use x509_cert::Certificate;

use super::authenticode::compute_hash;
use crate::error::{Result, SboltError};
use crate::keys::rsa2048;
use crate::pkcs7::build_authenticode_signed_data;

const SPC_INDIRECT_DATA_OBJID: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.3.6.1.4.1.311.2.1.4");
const SPC_PE_IMAGE_DATA_OBJID: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.3.6.1.4.1.311.2.1.15");
const SHA256_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
const WIN_CERT_REVISION_2_0: u16 = 0x0200;
const WIN_CERT_TYPE_PKCS_SIGNED_DATA: u16 = 0x0002;
const WIN_CERT_HEADER_SIZE: usize = 8;
const DER_SHORT_FORM_LENGTH_LIMIT: usize = 0x80;
const DER_ONE_BYTE_LENGTH_LIMIT: usize = 0x100;
const DER_TWO_BYTE_LENGTH_LIMIT: usize = 0x1_0000;
const PE_SIGNATURE_PREFIX_SIZE: usize = 4;
const CERT_TABLE_ENTRY_SIZE: usize = 4;
const PE_ALIGNMENT: usize = 8;

/// Sign a PE file with an Authenticode signature.
///
/// # Errors
///
/// Returns an error if hashing, CMS construction, or PE mutation fails.
pub fn sign(
    pe_data: &[u8],
    signer: &rsa2048::Signer,
    certificate: &Certificate,
) -> Result<Vec<u8>> {
    let hash = compute_hash(pe_data)?;

    let spc_content = build_spc_indirect_data(&hash)?;

    let pkcs7_der = build_authenticode_signed_data(
        SPC_INDIRECT_DATA_OBJID,
        &spc_content,
        &hash,
        signer,
        certificate,
    )?;

    let win_cert = build_win_certificate(&pkcs7_der)?;

    embed_signature(pe_data, &win_cert)
}

/// Build the inner fields of `SpcIndirectDataContent`.
///
/// Returns the concatenated DER of the two child elements **without** the
/// outer SEQUENCE wrapper. The caller is responsible for wrapping these
/// bytes in a SEQUENCE when constructing the `EncapsulatedContentInfo`.
fn build_spc_indirect_data(hash: &[u8; 32]) -> Result<Vec<u8>> {
    let mut result = Vec::new();

    let mut data_content = Vec::new();

    let sequence_tag = 0x30;
    let implicit_primitive_tag = 0x80;
    let constructed_tag = 0xa0;
    let constructed_2_tag = 0xa2;

    let oid_der = SPC_PE_IMAGE_DATA_OBJID
        .to_der()
        .map_err(|e| SboltError::Signing(format!("encode OID: {e}")))?;
    data_content.extend_from_slice(&oid_der);

    let mut spc_pe_image_data = Vec::new();

    spc_pe_image_data.extend_from_slice(&[0x03, 0x01, 0x00]);

    // This is the "<<<Obsolete>>>" string in UTF-16BE
    let obsolete_bmp: [u8; 28] = [
        0x00, 0x3c, 0x00, 0x3c, 0x00, 0x3c, 0x00, 0x4f, 0x00, 0x62, 0x00, 0x73, 0x00, 0x6f, 0x00,
        0x6c, 0x00, 0x65, 0x00, 0x74, 0x00, 0x65, 0x00, 0x3e, 0x00, 0x3e, 0x00, 0x3e,
    ];

    let mut unicode_field = Vec::new();
    unicode_field.push(implicit_primitive_tag);
    push_u8(&mut unicode_field, obsolete_bmp.len())?;
    unicode_field.extend_from_slice(&obsolete_bmp);

    let mut file_choice = Vec::new();
    file_choice.push(constructed_2_tag);
    push_u8(&mut file_choice, unicode_field.len())?;
    file_choice.extend_from_slice(&unicode_field);

    let mut spc_link = Vec::new();
    spc_link.push(constructed_tag);
    push_u8(&mut spc_link, file_choice.len())?;
    spc_link.extend_from_slice(&file_choice);

    spc_pe_image_data.extend_from_slice(&spc_link);

    let mut spc_pe_image_data_seq = Vec::new();
    spc_pe_image_data_seq.push(sequence_tag);
    encode_length(&mut spc_pe_image_data_seq, spc_pe_image_data.len())?;
    spc_pe_image_data_seq.extend_from_slice(&spc_pe_image_data);

    data_content.extend_from_slice(&spc_pe_image_data_seq);

    let mut data_seq = Vec::new();
    data_seq.push(sequence_tag);
    encode_length(&mut data_seq, data_content.len())?;
    data_seq.extend_from_slice(&data_content);

    result.extend_from_slice(&data_seq);

    let digest = OctetString::new(hash.to_vec())
        .map_err(|e| SboltError::Signing(format!("digest octet string: {e}")))?;

    let digest_info = DigestInfo {
        digest_algorithm: AlgorithmIdentifierOwned {
            oid: SHA256_OID,
            parameters: Some(Any::null()),
        },
        digest,
    };

    let digest_info_der = digest_info
        .to_der()
        .map_err(|e| SboltError::Signing(format!("encode digest info: {e}")))?;
    result.extend_from_slice(&digest_info_der);

    Ok(result)
}

/// `DigestInfo` structure.
#[derive(Clone, Debug, der::Sequence)]
struct DigestInfo {
    digest_algorithm: AlgorithmIdentifierOwned,
    digest: OctetString,
}

/// Encode ASN.1 length in DER format.
fn encode_length(buf: &mut Vec<u8>, len: usize) -> Result<()> {
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

/// Build `WIN_CERTIFICATE` for standard Authenticode (type 0x0002).
fn build_win_certificate(pkcs7_der: &[u8]) -> Result<Vec<u8>> {
    let total_size = checked_add(
        WIN_CERT_HEADER_SIZE,
        pkcs7_der.len(),
        "WIN_CERTIFICATE size",
    )?;
    let total_size_u32 = usize_to_u32(total_size, "WIN_CERTIFICATE")?;

    let mut result = Vec::with_capacity(total_size);

    result.extend_from_slice(&total_size_u32.to_le_bytes());
    result.extend_from_slice(&WIN_CERT_REVISION_2_0.to_le_bytes());
    result.extend_from_slice(&WIN_CERT_TYPE_PKCS_SIGNED_DATA.to_le_bytes());
    result.extend_from_slice(pkcs7_der);

    Ok(result)
}

/// Embed the signature into the PE file.
fn embed_signature(pe_data: &[u8], win_cert: &[u8]) -> Result<Vec<u8>> {
    let pe = PeFile64::parse(pe_data)
        .map_err(|_parse_error| SboltError::PeOperation("invalid or unsupported PE file".into()))?;

    let pe_offset = u32_to_usize(pe.dos_header().nt_headers_offset(), "PE header offset")?;
    let opt_offset = checked_add(
        pe_offset,
        PE_SIGNATURE_PREFIX_SIZE,
        "optional header offset",
    )?;
    let opt_offset = checked_add(
        opt_offset,
        size_of::<ImageFileHeader>(),
        "optional header offset",
    )?;
    let cert_table_index_offset = checked_mul(
        IMAGE_DIRECTORY_ENTRY_SECURITY,
        size_of::<ImageDataDirectory>(),
        "certificate table directory offset",
    )?;
    let cert_table_relative_offset = checked_add(
        size_of::<ImageOptionalHeader64>(),
        cert_table_index_offset,
        "certificate table directory offset",
    )?;
    let cert_table_dd_offset = checked_add(
        opt_offset,
        cert_table_relative_offset,
        "certificate table data directory offset",
    )?;
    let checksum_offset = checked_add(
        opt_offset,
        offset_of!(ImageOptionalHeader64, check_sum),
        "checksum field offset",
    )?;

    let aligned_size = align_to(pe_data.len(), PE_ALIGNMENT, "PE file alignment")?;

    let sig_aligned_size = align_to(win_cert.len(), PE_ALIGNMENT, "signature alignment")?;
    let sig_padding = sig_aligned_size
        .checked_sub(win_cert.len())
        .ok_or_else(|| SboltError::PeOperation("signature alignment underflow".into()))?;

    let result_capacity = checked_add(aligned_size, sig_aligned_size, "signed PE size")?;
    let mut result = Vec::with_capacity(result_capacity);
    result.extend_from_slice(pe_data);

    result.resize(aligned_size, 0);
    result.extend_from_slice(win_cert);
    let padded_len = checked_add(result.len(), sig_padding, "signed PE padding")?;
    result.resize(padded_len, 0);

    let aligned_size_u32 = usize_to_u32(aligned_size, "aligned PE size")?;
    let sig_aligned_size_u32 = usize_to_u32(sig_aligned_size, "signature size")?;

    write_u32_le(&mut result, cert_table_dd_offset, aligned_size_u32)?;
    let cert_table_size_offset = checked_add(
        cert_table_dd_offset,
        CERT_TABLE_ENTRY_SIZE,
        "certificate table size field",
    )?;
    write_u32_le(&mut result, cert_table_size_offset, sig_aligned_size_u32)?;

    let new_checksum = calculate_pe_checksum(&result, checksum_offset)?;
    write_u32_le(&mut result, checksum_offset, new_checksum)?;

    Ok(result)
}

/// Calculate the PE checksum.
fn calculate_pe_checksum(data: &[u8], checksum_offset: usize) -> Result<u32> {
    let mut sum: u64 = 0;

    let mut i = 0;
    while i < data.len() {
        if i == checksum_offset {
            i = checked_add(i, CERT_TABLE_ENTRY_SIZE, "checksum skip")?;
            continue;
        }

        let next_index = checked_add(i, 1, "checksum word end")?;
        let word = if next_index < data.len() {
            let bytes = data
                .get(i..=next_index)
                .ok_or_else(|| SboltError::PeOperation("checksum read beyond buffer".into()))?;
            let pair = read_checksum_pair(bytes)?;

            u64::from(u16::from_le_bytes(pair))
        } else {
            u64::from(
                *data
                    .get(i)
                    .ok_or_else(|| SboltError::PeOperation("checksum read beyond buffer".into()))?,
            )
        };

        sum = sum.wrapping_add(word);
        sum = fold_checksum(sum);

        i = checked_add(i, 2, "checksum iteration")?;
    }

    sum = fold_checksum(sum);

    let sum_u32 = u32::try_from(sum)
        .map_err(|_sum_error| SboltError::PeOperation("checksum exceeds 32-bit range".into()))?;
    let data_len_u32 = usize_to_u32(data.len(), "PE length")?;

    sum_u32
        .checked_add(data_len_u32)
        .ok_or_else(|| SboltError::PeOperation("checksum addition overflow".into()))
}

fn write_u32_le(data: &mut [u8], offset: usize, value: u32) -> Result<()> {
    let end = checked_add(offset, CERT_TABLE_ENTRY_SIZE, "write_u32 range end")?;

    data.get_mut(offset..end)
        .ok_or_else(|| SboltError::PeOperation("write beyond buffer".into()))
        .map(|bytes| bytes.copy_from_slice(&value.to_le_bytes()))
}

fn push_u8(buf: &mut Vec<u8>, value: usize) -> Result<()> {
    let byte = u8::try_from(value).map_err(|_conversion_error| {
        SboltError::Signing(format!("DER length byte out of range: {value}"))
    })?;
    buf.push(byte);

    Ok(())
}

fn usize_to_u32(value: usize, context: &str) -> Result<u32> {
    u32::try_from(value)
        .map_err(|_conversion_error| SboltError::PeOperation(format!("{context} exceeds u32")))
}

fn u32_to_usize(value: u32, context: &str) -> Result<usize> {
    usize::try_from(value)
        .map_err(|_conversion_error| SboltError::PeOperation(format!("{context} exceeds usize")))
}

fn checked_add(lhs: usize, rhs: usize, context: &str) -> Result<usize> {
    lhs.checked_add(rhs)
        .ok_or_else(|| SboltError::PeOperation(format!("{context} overflow")))
}

fn checked_mul(lhs: usize, rhs: usize, context: &str) -> Result<usize> {
    lhs.checked_mul(rhs)
        .ok_or_else(|| SboltError::PeOperation(format!("{context} overflow")))
}

fn align_to(value: usize, alignment: usize, context: &str) -> Result<usize> {
    let adjusted = checked_add(
        value,
        alignment
            .checked_sub(1)
            .ok_or_else(|| SboltError::PeOperation(format!("{context} invalid alignment")))?,
        context,
    )?;

    let alignment_mask = alignment
        .checked_sub(1)
        .ok_or_else(|| SboltError::PeOperation(format!("{context} invalid alignment")))?;

    Ok(adjusted & !alignment_mask)
}

fn fold_checksum(sum: u64) -> u64 {
    sum.wrapping_add(sum >> 16) & 0xffff
}

fn read_checksum_pair(bytes: &[u8]) -> Result<[u8; 2]> {
    bytes
        .try_into()
        .map_err(|_word_width_error| SboltError::PeOperation("invalid checksum word width".into()))
}

#[cfg(test)]
mod tests {
    use cms::content_info::ContentInfo;
    use cms::signed_data::SignedData;
    use der::asn1::OctetString;
    use der::{Decode, Encode};
    use ring::digest::{Context, SHA256};

    use super::*;
    use crate::keys::cert;
    use crate::keys::rsa2048;

    fn read_u32_le(data: &[u8], offset: usize) -> Result<u32> {
        data.get(offset..offset + 4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_le_bytes)
            .ok_or_else(|| SboltError::PeOperation("read beyond buffer".into()))
    }

    fn read_u16_le(data: &[u8], offset: usize) -> Result<u16> {
        data.get(offset..offset + 2)
            .and_then(|b| b.try_into().ok())
            .map(u16::from_le_bytes)
            .ok_or_else(|| SboltError::PeOperation("read beyond buffer".into()))
    }

    fn put_u16(buf: &mut Vec<u8>, offset: usize, val: u16) {
        buf[offset..offset + 2].copy_from_slice(&val.to_le_bytes());
    }

    fn put_u32(buf: &mut Vec<u8>, offset: usize, val: u32) {
        buf[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
    }

    /// Build a minimal PE32+ binary suitable for signing tests.
    fn build_signable_pe() -> Vec<u8> {
        let pe_offset: u32 = 0x40;
        let coff_offset = pe_offset as usize + 4;
        let opt_offset = coff_offset + 20;
        let opt_header_size: u16 = 240;
        let sections_offset = opt_offset + opt_header_size as usize;
        let headers_raw_end = sections_offset + 40;
        let file_alignment: u32 = 0x200;
        let size_of_headers =
            ((headers_raw_end as u32 + file_alignment - 1) / file_alignment) * file_alignment;

        let section_data: [u8; 512] = [0xCC; 512];
        let section_raw_offset = size_of_headers;
        let total_size = section_raw_offset as usize + section_data.len();
        let mut pe = vec![0u8; total_size];

        pe[0] = 0x4d;
        pe[1] = 0x5a;
        put_u32(&mut pe, 0x3c, pe_offset);

        pe[pe_offset as usize..pe_offset as usize + 4].copy_from_slice(&[0x50, 0x45, 0x00, 0x00]);

        put_u16(&mut pe, coff_offset, 0x8664);
        put_u16(&mut pe, coff_offset + 2, 1);
        put_u16(&mut pe, coff_offset + 16, opt_header_size);

        put_u16(&mut pe, opt_offset, 0x20b);
        put_u32(&mut pe, opt_offset + 60, size_of_headers);
        put_u32(&mut pe, opt_offset + 108, 16);

        pe[sections_offset..sections_offset + 6].copy_from_slice(b".text\0");
        put_u32(&mut pe, sections_offset + 16, section_data.len() as u32);
        put_u32(&mut pe, sections_offset + 20, section_raw_offset);

        pe[section_raw_offset as usize..section_raw_offset as usize + section_data.len()]
            .copy_from_slice(&section_data);

        pe
    }

    fn signer_and_cert() -> (rsa2048::Signer, Certificate) {
        let (pk_signer, pk_cert) = cert::generate_pk("Test PK").expect("generate PK cert");
        let (kek_signer, kek_cert) =
            cert::generate_kek("Test KEK", &pk_signer, &pk_cert).expect("generate KEK cert");
        let (db_signer, db_cert) =
            cert::generate_db("Test DB", &kek_signer, &kek_cert).expect("generate DB cert");
        (db_signer, db_cert)
    }

    #[test]
    fn sign_produces_valid_win_certificate() {
        // ARRANGE
        let pe = build_signable_pe();
        let (signer, cert) = signer_and_cert();

        // ACT
        let signed = sign(&pe, &signer, &cert).expect("sign should succeed");

        // ASSERT
        assert!(signed.len() > pe.len());

        let pe_offset = read_u32_le(&signed, 0x3c).unwrap() as usize;
        let opt_offset = pe_offset + 4 + 20;
        let dd_offset = opt_offset + 112;
        let cert_dd_offset = dd_offset + 4 * 8;

        let cert_addr = read_u32_le(&signed, cert_dd_offset).unwrap() as usize;
        let cert_size = read_u32_le(&signed, cert_dd_offset + 4).unwrap() as usize;
        assert!(cert_addr > 0, "cert table address must be set");
        assert!(cert_size > 8, "cert table must contain data");

        let dw_length = read_u32_le(&signed, cert_addr).unwrap();
        let w_revision = read_u16_le(&signed, cert_addr + 4).unwrap();
        let w_cert_type = read_u16_le(&signed, cert_addr + 6).unwrap();

        let aligned_dw_length = ((dw_length as usize) + 7) & !7;
        assert_eq!(
            aligned_dw_length, cert_size,
            "dwLength (aligned) must match DD size"
        );
        assert_eq!(
            w_revision, 0x0200,
            "wRevision must be WIN_CERT_REVISION_2_0"
        );
        assert_eq!(
            w_cert_type, 0x0002,
            "wCertificateType must be PKCS_SIGNED_DATA"
        );
    }

    #[test]
    fn sign_embeds_certificate_table() {
        // ARRANGE
        let pe = build_signable_pe();
        let (signer, cert) = signer_and_cert();

        // ACT
        let signed = sign(&pe, &signer, &cert).expect("sign should succeed");

        // ASSERT
        let pe_offset = read_u32_le(&signed, 0x3c).unwrap() as usize;
        let opt_offset = pe_offset + 4 + 20;
        let cert_dd_offset = opt_offset + 112 + 4 * 8;

        let cert_addr = read_u32_le(&signed, cert_dd_offset).unwrap() as usize;
        let cert_size = read_u32_le(&signed, cert_dd_offset + 4).unwrap() as usize;

        assert_eq!(cert_addr % 8, 0, "cert table must be 8-byte aligned");

        assert!(
            cert_addr + cert_size <= signed.len(),
            "cert table must be within file bounds"
        );
    }

    #[test]
    fn signed_pe_roundtrip_hash() {
        // ARRANGE
        let pe = build_signable_pe();
        let (signer, cert) = signer_and_cert();
        let hash_unsigned = compute_hash(&pe).expect("hash unsigned");

        // ACT
        let signed = sign(&pe, &signer, &cert).expect("sign");
        let hash_signed = compute_hash(&signed).expect("hash signed");

        // ASSERT
        assert_eq!(
            hash_unsigned, hash_signed,
            "authenticode hash must be identical before and after signing"
        );
    }

    #[test]
    fn message_digest_matches_econtent_value_octets() {
        // ARRANGE
        let pe = build_signable_pe();
        let (signer, cert) = signer_and_cert();

        // ACT
        let signed = sign(&pe, &signer, &cert).expect("sign");

        // Extract the PKCS#7 ContentInfo from the WIN_CERTIFICATE
        let pe_off = read_u32_le(&signed, 0x3c).unwrap() as usize;
        let opt_off = pe_off + 4 + 20;
        let cert_dd_off = opt_off + 112 + 4 * 8;
        let cert_addr = read_u32_le(&signed, cert_dd_off).unwrap() as usize;
        let dw_length = read_u32_le(&signed, cert_addr).unwrap() as usize;
        let pkcs7_bytes = &signed[cert_addr + 8..cert_addr + dw_length];

        let ci = ContentInfo::from_der(pkcs7_bytes).expect("parse ContentInfo");
        let sd = ci
            .content
            .decode_as::<SignedData>()
            .expect("decode SignedData");

        let econtent_any = sd
            .encap_content_info
            .econtent
            .as_ref()
            .expect("eContent must be present");

        let econtent_der = econtent_any.to_der().expect("econtent to der");
        assert_eq!(econtent_der[0], 0x30, "eContent must be a SEQUENCE");

        // ASSERT
        assert!(econtent_der[1] < 0x80);
        let hdr_len = 2;
        let value_octets = &econtent_der[hdr_len..];

        let signer_info = sd.signer_infos.0.iter().next().expect("signer info");
        let signed_attrs = signer_info.signed_attrs.as_ref().expect("signed attrs");

        let md_oid = const_oid::ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
        let md_attr = signed_attrs
            .iter()
            .find(|a| a.oid == md_oid)
            .expect("messageDigest attribute must exist");
        let md_value = md_attr.values.iter().next().expect("md value");
        let md_octet = md_value
            .decode_as::<OctetString>()
            .expect("decode md as OctetString");
        let message_digest = md_octet.as_bytes();

        let mut ctx = Context::new(&SHA256);
        ctx.update(value_octets);
        let computed = ctx.finish();

        assert_eq!(
            message_digest,
            computed.as_ref(),
            "messageDigest must equal SHA-256 of eContent value octets \
             (OVMF firmware strips the SEQUENCE tag+length before hashing)"
        );
    }

    #[test]
    fn build_spc_indirect_data_contains_sha256_digest() {
        // ARRANGE
        let hash = [0x5c_u8; 32];

        // ACT
        let spc = build_spc_indirect_data(&hash).expect("build SPC indirect data");

        // ASSERT
        assert!(spc.windows(hash.len()).any(|window| window == hash));
    }

    #[test]
    fn sign_rejects_invalid_pe_bytes() {
        // ARRANGE
        let (signer, cert) = signer_and_cert();

        // ACT
        let result = sign(b"not-a-pe-file", &signer, &cert);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn encode_length_supports_three_byte_values() {
        // ARRANGE
        let mut encoded = Vec::new();

        // ACT
        encode_length(&mut encoded, 0x01_0203).expect("encode length");

        // ASSERT
        assert_eq!(encoded, vec![0x83, 0x01, 0x02, 0x03]);
    }

    #[test]
    fn encode_length_supports_short_and_two_byte_values() {
        // ARRANGE
        let mut short = Vec::new();
        let mut one_byte = Vec::new();
        let mut two_byte = Vec::new();

        // ACT
        encode_length(&mut short, 0x7f).expect("encode short length");
        encode_length(&mut one_byte, 0x80).expect("encode one-byte length");
        encode_length(&mut two_byte, 0x1234).expect("encode two-byte length");

        // ASSERT
        assert_eq!(short, vec![0x7f]);
        assert_eq!(one_byte, vec![0x81, 0x80]);
        assert_eq!(two_byte, vec![0x82, 0x12, 0x34]);
    }

    #[test]
    fn encode_length_rejects_four_byte_values() {
        // ARRANGE
        let mut encoded = Vec::new();

        // ACT
        let result = encode_length(&mut encoded, 0x01_000000);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn push_u8_rejects_values_larger_than_byte() {
        // ARRANGE
        let mut encoded = Vec::new();

        // ACT
        let result = push_u8(&mut encoded, 0x100);

        // ASSERT
        assert!(result.is_err());
        assert!(encoded.is_empty());
    }

    #[test]
    fn build_win_certificate_layout_matches_header() {
        // ARRANGE
        let pkcs7 = b"pkcs7";

        // ACT
        let cert = build_win_certificate(pkcs7).expect("build WIN_CERTIFICATE");

        // ASSERT
        assert_eq!(
            u32::from_le_bytes(cert[0..4].try_into().expect("length")) as usize,
            cert.len()
        );
        assert_eq!(
            u16::from_le_bytes(cert[4..6].try_into().expect("revision")),
            0x0200
        );
        assert_eq!(
            u16::from_le_bytes(cert[6..8].try_into().expect("type")),
            0x0002
        );
        assert_eq!(&cert[8..], pkcs7);
    }

    #[test]
    fn embed_signature_rejects_non_pe32_plus_images() {
        // ARRANGE
        let mut pe = build_signable_pe();
        put_u16(&mut pe, 0x58, 0x10b);

        // ACT
        let result = embed_signature(&pe, b"cert");

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn embed_signature_rejects_invalid_pe_bytes() {
        // ARRANGE
        let pe = b"not-a-pe-file";

        // ACT
        let result = embed_signature(pe, b"cert");

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn embed_signature_aligns_input_and_certificate_sizes() {
        // ARRANGE
        let mut pe = build_signable_pe();
        pe.extend_from_slice(&[0xaa, 0xbb, 0xcc]);
        let win_cert = build_win_certificate(b"abc").expect("build WIN_CERTIFICATE");
        let aligned_pe_len = (pe.len() + 7) & !7;
        let aligned_cert_len = (win_cert.len() + 7) & !7;

        // ACT
        let signed = embed_signature(&pe, &win_cert).expect("embed signature");

        // ASSERT
        assert_eq!(signed.len(), aligned_pe_len + aligned_cert_len);
        assert_eq!(
            &signed[aligned_pe_len..aligned_pe_len + win_cert.len()],
            &win_cert
        );
    }

    #[test]
    fn embed_signature_preserves_original_section_payload() {
        // ARRANGE
        let pe = build_signable_pe();
        let win_cert = build_win_certificate(b"certificate").expect("build WIN_CERTIFICATE");
        let section_offset = 0x200;

        // ACT
        let signed = embed_signature(&pe, &win_cert).expect("embed signature");

        // ASSERT
        assert_eq!(&signed[section_offset..pe.len()], &pe[section_offset..]);
    }

    #[test]
    fn helper_alignment_and_write_success_paths_work() {
        // ARRANGE
        let mut data = [0_u8; 8];

        // ACT
        let aligned = align_to(9, 8, "align").expect("align length");
        write_u32_le(&mut data, 2, 0x1234_5678).expect("write u32");

        // ASSERT
        assert_eq!(aligned, 16);
        assert_eq!(&data[2..6], &0x1234_5678_u32.to_le_bytes());
    }

    #[test]
    fn align_to_rejects_adjustment_overflow() {
        // ARRANGE

        // ACT
        let result = align_to(usize::MAX, 8, "align");
        let zero_result = align_to(0, 8, "align");

        // ASSERT
        assert!(result.is_err());
        assert_eq!(zero_result.expect("align zero"), 0);
    }

    #[test]
    fn calculate_pe_checksum_handles_odd_length_and_skip_offset() {
        // ARRANGE
        let data = [1_u8, 2, 3, 4, 5];

        // ACT
        let checksum = calculate_pe_checksum(&data, 2).expect("calculate checksum");

        // ASSERT
        assert_eq!(checksum, 0x0201 + data.len() as u32);
    }

    #[test]
    fn checksum_and_bounds_helpers_validate_inputs() {
        // ARRANGE
        let checksum = calculate_pe_checksum(&[1_u8, 2, 3], 10).expect("checksum");

        // ACT
        let write_result = write_u32_le(&mut [0_u8; 3], 0, 1);
        let pair_result = read_checksum_pair(&[1_u8]);
        let conversion_result = usize_to_u32(u32::MAX as usize + 1, "too large");
        let offset_result = u32_to_usize(0, "offset");
        let add_result = checked_add(usize::MAX, 1, "add");
        let mul_result = checked_mul(usize::MAX, 2, "mul");
        let align_result = align_to(5, 0, "align");

        // ASSERT
        assert!(checksum > 0);
        assert!(write_result.is_err());
        assert!(pair_result.is_err());
        assert!(conversion_result.is_err());
        assert_eq!(offset_result.expect("convert offset"), 0);
        assert!(add_result.is_err());
        assert!(mul_result.is_err());
        assert!(align_result.is_err());
        assert_eq!(fold_checksum(0x12345), 0x2346);
    }
}
