//! PE signature embedding and PKCS#7 SignedData creation

use const_oid::ObjectIdentifier;
use der::{Encode, asn1::OctetString};
use spki::AlgorithmIdentifierOwned;
use x509_cert::Certificate;

use super::authenticode::compute_hash;
use crate::keys::Rsa2048Signer;
use crate::pe::PE32_PLUS_MAGIC;
use crate::pkcs7::build_authenticode_signed_data;
use crate::{Error, Result};

const SPC_INDIRECT_DATA_OBJID: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.3.6.1.4.1.311.2.1.4");
const SPC_PE_IMAGE_DATA_OBJID: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.3.6.1.4.1.311.2.1.15");
const SHA256_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
const WIN_CERT_REVISION_2_0: u16 = 0x0200;
const WIN_CERT_TYPE_PKCS_SIGNED_DATA: u16 = 0x0002;

/// Sign a PE file with Authenticode signature
pub fn sign(pe_data: &[u8], signer: &Rsa2048Signer, certificate: &Certificate) -> Result<Vec<u8>> {
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

/// Build the inner fields of SpcIndirectDataContent
///
/// Returns the concatenated DER of the two child elements **without** the
/// outer SEQUENCE wrapper. The caller is responsible for wrapping these
/// bytes in a SEQUENCE when constructing the `EncapsulatedContentInfo`
fn build_spc_indirect_data(hash: &[u8; 32]) -> Result<Vec<u8>> {
    let mut result = Vec::new();

    let mut data_content = Vec::new();

    let sequence_tag = 0x30;
    let implicit_primitive_tag = 0x80;
    let constructed_tag = 0xa0;
    let constructed_2_tag = 0xa2;

    let oid_der = SPC_PE_IMAGE_DATA_OBJID
        .to_der()
        .map_err(|e| Error::Signing(format!("encode OID: {e}")))?;
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
    unicode_field.push(obsolete_bmp.len() as u8);
    unicode_field.extend_from_slice(&obsolete_bmp);

    let mut file_choice = Vec::new();
    file_choice.push(constructed_2_tag);
    file_choice.push(unicode_field.len() as u8);
    file_choice.extend_from_slice(&unicode_field);

    let mut spc_link = Vec::new();
    spc_link.push(constructed_tag);
    spc_link.push(file_choice.len() as u8);
    spc_link.extend_from_slice(&file_choice);

    spc_pe_image_data.extend_from_slice(&spc_link);

    let mut spc_pe_image_data_seq = Vec::new();
    spc_pe_image_data_seq.push(sequence_tag);
    encode_length(&mut spc_pe_image_data_seq, spc_pe_image_data.len());
    spc_pe_image_data_seq.extend_from_slice(&spc_pe_image_data);

    data_content.extend_from_slice(&spc_pe_image_data_seq);

    let mut data_seq = Vec::new();
    data_seq.push(sequence_tag);
    encode_length(&mut data_seq, data_content.len());
    data_seq.extend_from_slice(&data_content);

    result.extend_from_slice(&data_seq);

    let digest = OctetString::new(hash.to_vec())
        .map_err(|e| Error::Signing(format!("digest octet string: {e}")))?;

    let digest_info = DigestInfo {
        digest_algorithm: AlgorithmIdentifierOwned {
            oid: SHA256_OID,
            parameters: Some(der::asn1::Any::null()),
        },
        digest,
    };

    let digest_info_der = digest_info
        .to_der()
        .map_err(|e| Error::Signing(format!("encode digest info: {e}")))?;
    result.extend_from_slice(&digest_info_der);

    Ok(result)
}

/// DigestInfo structure
#[derive(Clone, Debug, der::Sequence)]
struct DigestInfo {
    digest_algorithm: AlgorithmIdentifierOwned,
    digest: OctetString,
}

/// Encode ASN.1 length in DER format
fn encode_length(buf: &mut Vec<u8>, len: usize) {
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

/// Build WIN_CERTIFICATE structure for standard Authenticode (type 0x0002)
fn build_win_certificate(pkcs7_der: &[u8]) -> Result<Vec<u8>> {
    let header_size = 8; // 4 + 2 + 2
    let total_size = header_size + pkcs7_der.len();

    let mut result = Vec::with_capacity(total_size);

    result.extend_from_slice(&(total_size as u32).to_le_bytes());
    result.extend_from_slice(&WIN_CERT_REVISION_2_0.to_le_bytes());
    result.extend_from_slice(&WIN_CERT_TYPE_PKCS_SIGNED_DATA.to_le_bytes());
    result.extend_from_slice(pkcs7_der);

    Ok(result)
}

/// Embed the signature into the PE file
fn embed_signature(pe_data: &[u8], win_cert: &[u8]) -> Result<Vec<u8>> {
    let pe_offset = read_u32_le(pe_data, 0x3c)? as usize;
    let coff_offset = pe_offset + 4;
    let opt_offset = coff_offset + 20;

    let magic = read_u16_le(pe_data, opt_offset)?;
    if magic != PE32_PLUS_MAGIC {
        return Err(Error::PeOperation("only PE32+ is supported".into()));
    }

    let dd_offset = opt_offset + 112;
    let cert_table_dd_offset = dd_offset + (4 * 8); // DD[4]

    let aligned_size = (pe_data.len() + 7) & !7;

    let sig_aligned_size = (win_cert.len() + 7) & !7;
    let sig_padding = sig_aligned_size - win_cert.len();

    let mut result = Vec::with_capacity(aligned_size + sig_aligned_size);
    result.extend_from_slice(pe_data);

    result.resize(aligned_size, 0);
    result.extend_from_slice(win_cert);
    result.resize(result.len() + sig_padding, 0);

    write_u32_le(&mut result, cert_table_dd_offset, aligned_size as u32)?;
    write_u32_le(
        &mut result,
        cert_table_dd_offset + 4,
        sig_aligned_size as u32,
    )?;

    let checksum_offset = opt_offset + 64;
    let new_checksum = calculate_pe_checksum(&result, checksum_offset);
    write_u32_le(&mut result, checksum_offset, new_checksum)?;

    Ok(result)
}

/// Calculate PE checksum
fn calculate_pe_checksum(data: &[u8], checksum_offset: usize) -> u32 {
    let mut sum: u64 = 0;

    let mut i = 0;
    while i < data.len() {
        // Skip checksum field
        if i == checksum_offset {
            i += 4;
            continue;
        }

        let word = if i + 1 < data.len() {
            u16::from_le_bytes([data[i], data[i + 1]]) as u64
        } else {
            data[i] as u64
        };

        sum += word;
        // Fold carries
        sum = (sum & 0xffff) + (sum >> 16);

        i += 2;
    }

    // Fold final carry
    sum = (sum & 0xffff) + (sum >> 16);

    // Add file size
    (sum as u32) + (data.len() as u32)
}

fn read_u16_le(data: &[u8], offset: usize) -> Result<u16> {
    if offset + 2 > data.len() {
        return Err(Error::PeOperation("read beyond buffer".into()));
    }
    Ok(u16::from_le_bytes([data[offset], data[offset + 1]]))
}

fn read_u32_le(data: &[u8], offset: usize) -> Result<u32> {
    if offset + 4 > data.len() {
        return Err(Error::PeOperation("read beyond buffer".into()));
    }
    Ok(u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]))
}

fn write_u32_le(data: &mut [u8], offset: usize, value: u32) -> Result<()> {
    if offset + 4 > data.len() {
        return Err(Error::PeOperation("write beyond buffer".into()));
    }
    let bytes = value.to_le_bytes();
    data[offset..offset + 4].copy_from_slice(&bytes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use cms::content_info::ContentInfo;
    use cms::signed_data::SignedData;
    use der::asn1::OctetString;
    use der::{Decode, Encode};
    use ring::digest::{Context, SHA256};

    use super::*;
    use crate::keys::{
        Rsa2048Signer, generate_db_certificate, generate_kek_certificate, generate_pk_certificate,
    };

    /// Write a little-endian u16 into a buffer.
    fn put_u16(buf: &mut Vec<u8>, offset: usize, val: u16) {
        buf[offset..offset + 2].copy_from_slice(&val.to_le_bytes());
    }

    /// Write a little-endian u32 into a buffer.
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

    /// Create a test signer and certificate.
    fn test_signer_and_cert() -> (Rsa2048Signer, Certificate) {
        let (pk_signer, pk_cert) = generate_pk_certificate("Test PK").expect("generate PK cert");
        let (kek_signer, kek_cert) =
            generate_kek_certificate("Test KEK", &pk_signer, &pk_cert).expect("generate KEK cert");
        let (db_signer, db_cert) =
            generate_db_certificate("Test DB", &kek_signer, &kek_cert).expect("generate DB cert");
        (db_signer, db_cert)
    }

    #[test]
    fn test_sign_produces_valid_win_certificate() {
        let pe = build_signable_pe();
        let (signer, cert) = test_signer_and_cert();

        let signed = sign(&pe, &signer, &cert).expect("sign should succeed");

        // The signed PE must be larger than the original
        assert!(signed.len() > pe.len());

        // Read Certificate Table DD[4] to find the WIN_CERTIFICATE
        let pe_offset = read_u32_le(&signed, 0x3c).unwrap() as usize;
        let opt_offset = pe_offset + 4 + 20;
        let dd_offset = opt_offset + 112;
        let cert_dd_offset = dd_offset + 4 * 8;

        let cert_addr = read_u32_le(&signed, cert_dd_offset).unwrap() as usize;
        let cert_size = read_u32_le(&signed, cert_dd_offset + 4).unwrap() as usize;
        assert!(cert_addr > 0, "cert table address must be set");
        assert!(cert_size > 8, "cert table must contain data");

        // Verify WIN_CERTIFICATE header fields
        let dw_length = read_u32_le(&signed, cert_addr).unwrap();
        let w_revision = read_u16_le(&signed, cert_addr + 4).unwrap();
        let w_cert_type = read_u16_le(&signed, cert_addr + 6).unwrap();

        // dwLength is the unpadded WIN_CERTIFICATE size; DD stores the 8-byte aligned size
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
    fn test_sign_embeds_certificate_table() {
        let pe = build_signable_pe();
        let (signer, cert) = test_signer_and_cert();

        let signed = sign(&pe, &signer, &cert).expect("sign should succeed");

        // Verify DD[4] is updated
        let pe_offset = read_u32_le(&signed, 0x3c).unwrap() as usize;
        let opt_offset = pe_offset + 4 + 20;
        let cert_dd_offset = opt_offset + 112 + 4 * 8;

        let cert_addr = read_u32_le(&signed, cert_dd_offset).unwrap() as usize;
        let cert_size = read_u32_le(&signed, cert_dd_offset + 4).unwrap() as usize;

        // cert_addr must be 8-byte aligned
        assert_eq!(cert_addr % 8, 0, "cert table must be 8-byte aligned");

        // cert data must fit within the signed PE
        assert!(
            cert_addr + cert_size <= signed.len(),
            "cert table must be within file bounds"
        );
    }

    #[test]
    fn test_signed_pe_roundtrip_hash() {
        let pe = build_signable_pe();
        let (signer, cert) = test_signer_and_cert();

        // Hash the unsigned PE
        let hash_unsigned = compute_hash(&pe).expect("hash unsigned");

        // Sign the PE
        let signed = sign(&pe, &signer, &cert).expect("sign");

        // Hash the signed PE (should exclude the certificate table)
        let hash_signed = compute_hash(&signed).expect("hash signed");

        assert_eq!(
            hash_unsigned, hash_signed,
            "authenticode hash must be identical before and after signing"
        );
    }

    #[test]
    fn test_message_digest_matches_econtent_value_octets() {
        let pe = build_signable_pe();
        let (signer, cert) = test_signer_and_cert();
        let signed = sign(&pe, &signer, &cert).expect("sign");

        // Extract the PKCS#7 ContentInfo from the WIN_CERTIFICATE
        let pe_off = read_u32_le(&signed, 0x3c).unwrap() as usize;
        let opt_off = pe_off + 4 + 20;
        let cert_dd_off = opt_off + 112 + 4 * 8;
        let cert_addr = read_u32_le(&signed, cert_dd_off).unwrap() as usize;
        let dw_length = read_u32_le(&signed, cert_addr).unwrap() as usize;
        let pkcs7_bytes = &signed[cert_addr + 8..cert_addr + dw_length];

        // Parse ContentInfo -> SignedData
        let ci = ContentInfo::from_der(pkcs7_bytes).expect("parse ContentInfo");
        let sd = ci
            .content
            .decode_as::<SignedData>()
            .expect("decode SignedData");

        // Get eContent (should be a SEQUENCE containing the inner fields).
        let econtent_any = sd
            .encap_content_info
            .econtent
            .as_ref()
            .expect("eContent must be present");

        // The eContent Any is a SEQUENCE. Get its value bytes (the inner
        // fields without the SEQUENCE tag+length). This is what OVMF
        // passes to OpenSSL's PKCS7_verify.
        let econtent_der = econtent_any.to_der().expect("econtent to der");
        assert_eq!(econtent_der[0], 0x30, "eContent must be a SEQUENCE");

        // Strip the SEQUENCE tag+length to get the value octets.
        // DER SEQUENCE tag is 0x30, followed by length encoding.
        assert_eq!(econtent_der[0], 0x30);
        let (hdr_len, _content_len) = if econtent_der[1] < 0x80 {
            (2, econtent_der[1] as usize)
        } else if econtent_der[1] == 0x81 {
            (3, econtent_der[2] as usize)
        } else if econtent_der[1] == 0x82 {
            (
                4,
                ((econtent_der[2] as usize) << 8) | econtent_der[3] as usize,
            )
        } else {
            panic!("unexpected DER length encoding");
        };
        let value_octets = &econtent_der[hdr_len..];

        // Extract messageDigest from signer info's signed attributes
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

        // Compute SHA-256 of the value octets
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
}
