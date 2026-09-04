//! PE signature embedding and PKCS#7 `SignedData` creation.

mod ops;
mod spc;

use core::mem::{offset_of, size_of};
use std::io::{Read, Write};

use der::Encode as _;
use object::pe::{IMAGE_DIRECTORY_ENTRY_SECURITY, ImageDataDirectory, ImageOptionalHeader64};
use ops::{CERT_TABLE_ENTRY_SIZE, build_win_certificate, hash_range_excluding, put_u32_le};
use sha2::{Digest as _, Sha256};
use spc::build_spc_indirect_data;
use uki::align;
use uki::metadata;
use x509_cert::Certificate;

use crate::error::{Result, SboltError};
use crate::keys::rsa2048;
use crate::pkcs7;

pub(super) const SPC_INDIRECT_DATA_OBJID: const_oid::ObjectIdentifier =
    const_oid::ObjectIdentifier::new_unwrap("1.3.6.1.4.1.311.2.1.4");
pub(super) const PE_ALIGNMENT: usize = 8;

/// Exact size of the `WIN_CERTIFICATE` table appended by [`sign`], aligned to
/// 8 bytes.
///
/// # Errors
///
/// Returns an error when the certificate table size cannot be computed.
pub fn cert_table_size(certificate: &Certificate) -> Result<usize> {
    let dummy_spc = build_spc_indirect_data(&[0_u8; 32])?;

    pkcs7::compute_authenticode_size(SPC_INDIRECT_DATA_OBJID, &dummy_spc, certificate)
}

/// Sign a PE file with an Authenticode signature.
///
/// # Errors
///
/// Returns an error when the PE is malformed, hashing fails, or the writer
/// rejects bytes.
pub fn sign<R: Read, W: Write>(
    reader: &mut R,
    signer: &rsa2048::Signer,
    certificate: &Certificate,
    writer: &mut W,
) -> Result<()> {
    let (mut header_buf, meta) =
        metadata::extract(reader).map_err(|e| SboltError::PeOperation(format!("{e}")))?;
    let size_of_headers = usize::try_from(meta.size_of_headers)
        .map_err(|e| SboltError::PeOperation(format!("SizeOfHeaders exceeds usize: {e}")))?;

    let num_dirs = usize::try_from(meta.num_data_directories)
        .map_err(|e| SboltError::PeOperation(format!("directory count exceeds usize: {e}")))?;
    if num_dirs <= IMAGE_DIRECTORY_ENTRY_SECURITY {
        return Err(SboltError::PeOperation(
            "no certificate table data directory".into(),
        ));
    }

    let checksum_offset = meta
        .optional_header_offset
        .checked_add(offset_of!(ImageOptionalHeader64, check_sum))
        .ok_or_else(|| SboltError::PeOperation("checksum field offset overflow".into()))?;
    let dd_offset = meta
        .optional_header_offset
        .checked_add(size_of::<ImageOptionalHeader64>())
        .ok_or_else(|| SboltError::PeOperation("data directory offset overflow".into()))?;
    let cert_dir = IMAGE_DIRECTORY_ENTRY_SECURITY
        .checked_mul(size_of::<ImageDataDirectory>())
        .ok_or_else(|| SboltError::PeOperation("cert dir index overflow".into()))?;
    let cert_table_dd_offset = dd_offset
        .checked_add(cert_dir)
        .ok_or_else(|| SboltError::PeOperation("cert dir offset overflow".into()))?;

    let overflow = if header_buf.len() > size_of_headers {
        header_buf.split_off(size_of_headers)
    } else {
        Vec::new()
    };
    header_buf.truncate(size_of_headers);

    let pe_size = usize::try_from(meta.last_section_file_end)
        .map_err(|e| SboltError::PeOperation(format!("PE size exceeds usize: {e}")))?;

    let cert_size = cert_table_size(certificate)?;

    let cert_table_va = align::up(pe_size, PE_ALIGNMENT)
        .map_err(|_source| SboltError::PeOperation("cert table VA overflow".into()))?;
    let cert_table_va_u32 = u32::try_from(cert_table_va)
        .map_err(|e| SboltError::PeOperation(format!("cert table VA exceeds u32: {e}")))?;
    let cert_size_u32 = u32::try_from(cert_size)
        .map_err(|e| SboltError::PeOperation(format!("cert size exceeds u32: {e}")))?;
    put_u32_le(&mut header_buf, checksum_offset, 0_u32)?;
    put_u32_le(&mut header_buf, cert_table_dd_offset, cert_table_va_u32)?;
    let cert_table_size_offset = cert_table_dd_offset
        .checked_add(CERT_TABLE_ENTRY_SIZE)
        .ok_or_else(|| SboltError::PeOperation("cert table size offset overflow".into()))?;
    put_u32_le(&mut header_buf, cert_table_size_offset, cert_size_u32)?;

    let mut hash_ctx = Sha256::new();
    hash_range_excluding(
        &mut hash_ctx,
        header_buf.as_slice(),
        0,
        size_of_headers,
        &[
            (checksum_offset, size_of::<u32>()),
            (cert_table_dd_offset, size_of::<ImageDataDirectory>()),
        ],
    )?;

    writer
        .write_all(header_buf.as_slice())
        .map_err(|e| SboltError::Signing(format!("write headers: {e}")))?;
    drop(header_buf);

    let mut section_written = 0_usize;
    if !overflow.is_empty() {
        hash_ctx.update(overflow.as_slice());
        writer
            .write_all(overflow.as_slice())
            .map_err(|e| SboltError::Signing(format!("write overflow: {e}")))?;
        section_written = overflow.len();
    }

    let expected = pe_size.saturating_sub(size_of_headers.saturating_add(section_written));
    let streamed = stream_sections(reader, writer, &mut hash_ctx, expected)?;
    section_written = section_written.saturating_add(streamed);

    let mut chunk = vec![0_u8; 4096];
    loop {
        let n = reader
            .read(chunk.as_mut_slice())
            .map_err(|e| SboltError::Signing(format!("read trailing: {e}")))?;
        if n == 0 {
            break;
        }
        let data = chunk
            .get(..n)
            .ok_or_else(|| SboltError::Signing("trailing slice out of bounds".into()))?;
        hash_ctx.update(data);
        writer
            .write_all(data)
            .map_err(|e| SboltError::Signing(format!("write trailing: {e}")))?;
        section_written = section_written.saturating_add(n);
    }

    let written = size_of_headers.saturating_add(section_written);

    build_and_write_signature(hash_ctx, signer, certificate, writer, written)
}

/// Stream section data from reader, feeding into hash context and writer.
fn stream_sections<W: Write>(
    reader: &mut dyn Read,
    writer: &mut W,
    hash: &mut Sha256,
    expected: usize,
) -> Result<usize> {
    let mut remaining = expected;
    let mut total_written = 0_usize;
    let mut chunk = vec![0_u8; 4096];
    while remaining > 0 {
        let to_read = remaining.min(chunk.len());
        let buf = chunk
            .get_mut(..to_read)
            .ok_or_else(|| SboltError::Signing("chunk slicing failed".into()))?;
        let n = reader
            .read(buf)
            .map_err(|e| SboltError::Signing(format!("read sections: {e}")))?;
        if n == 0 {
            return Err(SboltError::PeOperation("truncated PE section data".into()));
        }
        hash.update(
            chunk
                .get(..n)
                .ok_or_else(|| SboltError::Signing("chunk slicing failed".into()))?,
        );
        writer
            .write_all(
                chunk
                    .get(..n)
                    .ok_or_else(|| SboltError::Signing("chunk slicing failed".into()))?,
            )
            .map_err(|e| SboltError::Signing(format!("write sections: {e}")))?;
        remaining = remaining.saturating_sub(n);
        total_written = total_written.saturating_add(n);
    }

    Ok(total_written)
}

/// Build the Authenticode signature and write the `WIN_CERTIFICATE` with padding.
fn build_and_write_signature<W: Write>(
    hash_ctx: Sha256,
    signer: &rsa2048::Signer,
    certificate: &Certificate,
    writer: &mut W,
    written: usize,
) -> Result<()> {
    let digest = hash_ctx.finalize();
    let mut hash = [0_u8; 32];
    hash.copy_from_slice(&digest);
    let spc_content = build_spc_indirect_data(&hash)?;
    let digest = Sha256::digest(&spc_content);
    let mut digest_bytes = [0_u8; 32];
    digest_bytes.copy_from_slice(&digest);
    let signed_attrs = pkcs7::build_signed_attributes(SPC_INDIRECT_DATA_OBJID, &digest_bytes)?;
    let attrs_der = signed_attrs
        .to_der()
        .map_err(|e| SboltError::Signing(format!("encode signed attrs: {e}")))?;
    let sig = signer.sign_pkcs1v15_sha256(&attrs_der)?;

    let pkcs7_der = pkcs7::build_authenticode_signed_data(
        SPC_INDIRECT_DATA_OBJID,
        &spc_content,
        &sig,
        certificate,
        Some(signed_attrs),
    )?;

    let win_cert = build_win_certificate(&pkcs7_der)?;

    let aligned = align::up(written, PE_ALIGNMENT)
        .map_err(|_source| SboltError::PeOperation("cert table alignment overflow".into()))?;
    let padding = aligned.saturating_sub(written);
    if padding > 0 {
        let pad_bytes = [0_u8; 7]
            .get(..padding)
            .ok_or_else(|| SboltError::Signing("padding slice out of bounds".into()))?;
        writer
            .write_all(pad_bytes)
            .map_err(|e| SboltError::Signing(format!("write cert padding: {e}")))?;
    }

    writer
        .write_all(&win_cert)
        .map_err(|e| SboltError::Signing(format!("write WIN_CERTIFICATE: {e}")))?;

    let cert_aligned = align::up(win_cert.len(), PE_ALIGNMENT)
        .map_err(|_source| SboltError::PeOperation("cert alignment overflow".into()))?;
    let cert_pad = cert_aligned.saturating_sub(win_cert.len());
    if cert_pad > 0 {
        let pad_bytes = [0_u8; 7]
            .get(..cert_pad)
            .ok_or_else(|| SboltError::Signing("cert padding slice out of bounds".into()))?;
        writer
            .write_all(pad_bytes)
            .map_err(|e| SboltError::Signing(format!("write cert padding: {e}")))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use cms::content_info::ContentInfo;
    use cms::signed_data::SignedData;
    use der::Decode as _;
    use der::asn1::OctetString;
    use object::pe::IMAGE_NT_OPTIONAL_HDR64_MAGIC;
    use sha2::Sha256;

    use super::*;
    use crate::keys::cert;
    use crate::keys::rsa2048;

    fn read_u32_le(data: &[u8], offset: usize) -> Result<u32> {
        let end = offset
            .checked_add(4)
            .ok_or_else(|| SboltError::PeOperation("read beyond buffer".into()))?;
        data.get(offset..end)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes)
            .ok_or_else(|| SboltError::PeOperation("read beyond buffer".into()))
    }

    fn read_u16_le(data: &[u8], offset: usize) -> Result<u16> {
        let end = offset
            .checked_add(2)
            .ok_or_else(|| SboltError::PeOperation("read beyond buffer".into()))?;
        data.get(offset..end)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u16::from_le_bytes)
            .ok_or_else(|| SboltError::PeOperation("read beyond buffer".into()))
    }

    fn put_u16(buf: &mut [u8], offset: usize, val: u16) {
        let end = offset.checked_add(2).expect("u16 write end");
        buf.get_mut(offset..end)
            .expect("u16 write bytes")
            .copy_from_slice(&val.to_le_bytes());
    }

    fn put_u32(buf: &mut [u8], offset: usize, val: u32) {
        let end = offset.checked_add(4).expect("u32 write end");
        buf.get_mut(offset..end)
            .expect("u32 write bytes")
            .copy_from_slice(&val.to_le_bytes());
    }

    fn build_signable_pe() -> Vec<u8> {
        let pe_offset: u32 = 0x40;
        let pe_offset_usize = usize::try_from(pe_offset).expect("PE offset fits usize");
        let coff_offset = pe_offset_usize.checked_add(4).expect("COFF offset");
        let opt_offset = coff_offset.checked_add(20).expect("optional header offset");
        let opt_header_size: u16 = 240;
        let sections_offset = opt_offset
            .checked_add(usize::from(opt_header_size))
            .expect("sections offset");
        let headers_raw_end = sections_offset.checked_add(40).expect("headers raw end");
        let file_alignment: u32 = 0x200;
        let size_of_headers = u32::try_from(headers_raw_end)
            .expect("headers size fits u32")
            .div_ceil(file_alignment)
            .checked_mul(file_alignment)
            .expect("headers size overflow");

        let section_data: [u8; 512] = [0xCC; 512];
        let section_raw_offset = size_of_headers;
        let section_raw_offset_usize =
            usize::try_from(section_raw_offset).expect("section offset fits usize");
        let total_size = section_raw_offset_usize
            .checked_add(section_data.len())
            .expect("total PE size");
        let mut pe = vec![0_u8; total_size];

        *pe.get_mut(0).expect("DOS magic byte") = 0x4d;
        *pe.get_mut(1).expect("DOS magic byte") = 0x5a;
        put_u32(&mut pe, 0x3c, pe_offset);

        let pe_signature_end = pe_offset_usize.checked_add(4).expect("PE signature end");
        pe.get_mut(pe_offset_usize..pe_signature_end)
            .expect("PE signature bytes")
            .copy_from_slice(&[0x50, 0x45, 0x00, 0x00]);

        put_u16(&mut pe, coff_offset, 0x8664);
        put_u16(
            &mut pe,
            coff_offset.checked_add(2).expect("section count offset"),
            1,
        );
        put_u16(
            &mut pe,
            coff_offset
                .checked_add(16)
                .expect("optional header size offset"),
            opt_header_size,
        );

        put_u16(&mut pe, opt_offset, IMAGE_NT_OPTIONAL_HDR64_MAGIC);
        put_u32(
            &mut pe,
            opt_offset
                .checked_add(32)
                .expect("section alignment offset"),
            0x1000,
        );
        put_u32(
            &mut pe,
            opt_offset.checked_add(36).expect("file alignment offset"),
            file_alignment,
        );
        put_u32(
            &mut pe,
            opt_offset.checked_add(60).expect("headers size offset"),
            size_of_headers,
        );
        put_u32(
            &mut pe,
            opt_offset.checked_add(108).expect("directory count offset"),
            16,
        );

        let section_name_end = sections_offset.checked_add(6).expect("section name end");
        pe.get_mut(sections_offset..section_name_end)
            .expect("section name bytes")
            .copy_from_slice(b".text\0");
        put_u32(
            &mut pe,
            sections_offset
                .checked_add(16)
                .expect("section size offset"),
            u32::try_from(section_data.len()).expect("section size fits u32"),
        );
        put_u32(
            &mut pe,
            sections_offset.checked_add(20).expect("section raw offset"),
            section_raw_offset,
        );

        let section_data_end = section_raw_offset_usize
            .checked_add(section_data.len())
            .expect("section data end");
        pe.get_mut(section_raw_offset_usize..section_data_end)
            .expect("section data bytes")
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
        let mut signed_pe = Vec::new();
        sign(&mut pe.as_slice(), &signer, &cert, &mut signed_pe).expect("sign should succeed");

        // ASSERT
        assert!(signed_pe.len() > pe.len());

        let pe_offset = usize::try_from(read_u32_le(&signed_pe, 0x3c).expect("read PE offset"))
            .expect("PE offset fits usize");
        let opt_offset = pe_offset
            .checked_add(4)
            .expect("optional header signature offset")
            .checked_add(20)
            .expect("optional header offset");
        let dd_offset = opt_offset.checked_add(112).expect("data directory offset");
        let cert_dd_offset = dd_offset
            .checked_add(
                4_usize
                    .checked_mul(8)
                    .expect("certificate directory offset"),
            )
            .expect("certificate directory offset");

        let cert_addr =
            usize::try_from(read_u32_le(&signed_pe, cert_dd_offset).expect("read cert address"))
                .expect("cert address fits usize");
        let cert_size = usize::try_from(
            read_u32_le(
                &signed_pe,
                cert_dd_offset.checked_add(4).expect("cert size offset"),
            )
            .expect("read cert size"),
        )
        .expect("cert size fits usize");
        assert!(cert_addr > 0, "cert table address must be set");
        assert!(cert_size > 8, "cert table must contain data");

        let dw_length = read_u32_le(&signed_pe, cert_addr).expect("read cert length");
        let w_revision = read_u16_le(
            &signed_pe,
            cert_addr.checked_add(4).expect("revision offset"),
        )
        .expect("read revision");
        let w_cert_type = read_u16_le(
            &signed_pe,
            cert_addr.checked_add(6).expect("cert type offset"),
        )
        .expect("read cert type");

        let aligned_dw_length = usize::try_from(dw_length)
            .expect("cert length fits usize")
            .checked_add(7)
            .expect("aligned cert length")
            & !7;
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
        let mut signed_pe = Vec::new();
        sign(&mut pe.as_slice(), &signer, &cert, &mut signed_pe).expect("sign should succeed");

        // ASSERT
        let pe_offset = usize::try_from(read_u32_le(&signed_pe, 0x3c).expect("read PE offset"))
            .expect("PE offset fits usize");
        let opt_offset = pe_offset
            .checked_add(4)
            .expect("optional header signature offset")
            .checked_add(20)
            .expect("optional header offset");
        let cert_dd_offset = opt_offset
            .checked_add(112)
            .expect("data directory offset")
            .checked_add(
                4_usize
                    .checked_mul(8)
                    .expect("certificate directory offset"),
            )
            .expect("certificate directory offset");

        let cert_addr =
            usize::try_from(read_u32_le(&signed_pe, cert_dd_offset).expect("read cert address"))
                .expect("cert address fits usize");
        let cert_size = usize::try_from(
            read_u32_le(
                &signed_pe,
                cert_dd_offset.checked_add(4).expect("cert size offset"),
            )
            .expect("read cert size"),
        )
        .expect("cert size fits usize");

        assert_eq!(cert_addr & 7, 0, "cert table must be 8-byte aligned");

        assert!(
            cert_addr.checked_add(cert_size).expect("cert table end") <= signed_pe.len(),
            "cert table must be within file bounds"
        );
    }

    #[test]
    fn cert_table_size_matches_signed_output_growth() {
        // ARRANGE
        let pe = build_signable_pe();
        let (signer, cert) = signer_and_cert();
        let table_size = cert_table_size(&cert).expect("cert table size");

        // ACT
        let mut signed_pe = Vec::new();
        sign(&mut pe.as_slice(), &signer, &cert, &mut signed_pe).expect("sign should succeed");

        // ASSERT
        let aligned_unsigned = pe.len().checked_add(7).expect("align bound") & !7;
        assert_eq!(
            signed_pe.len().saturating_sub(aligned_unsigned),
            table_size,
            "signed size must be align8(unsigned) + cert_table_size"
        );
    }

    #[test]
    fn message_digest_matches_econtent_value_octets() {
        // ARRANGE
        let pe = build_signable_pe();
        let (signer, cert) = signer_and_cert();

        // ACT
        let mut signed_pe = Vec::new();
        sign(&mut pe.as_slice(), &signer, &cert, &mut signed_pe).expect("sign");

        // Extract the PKCS#7 ContentInfo from the WIN_CERTIFICATE
        let pe_offset = usize::try_from(read_u32_le(&signed_pe, 0x3c).expect("read PE offset"))
            .expect("PE offset fits usize");
        let opt_offset = pe_offset
            .checked_add(4)
            .expect("optional header signature offset")
            .checked_add(20)
            .expect("optional header offset");
        let cert_directory_offset = opt_offset
            .checked_add(112)
            .expect("data directory offset")
            .checked_add(
                4_usize
                    .checked_mul(8)
                    .expect("certificate directory offset"),
            )
            .expect("certificate directory offset");
        let cert_addr = usize::try_from(
            read_u32_le(&signed_pe, cert_directory_offset).expect("read cert address"),
        )
        .expect("cert address fits usize");
        let cert_length =
            usize::try_from(read_u32_le(&signed_pe, cert_addr).expect("read cert length"))
                .expect("cert length fits usize");
        let pkcs7_offset = cert_addr.checked_add(8).expect("PKCS#7 offset");
        let pkcs7_end = cert_addr.checked_add(cert_length).expect("PKCS#7 end");
        let pkcs7_bytes = signed_pe
            .get(pkcs7_offset..pkcs7_end)
            .expect("PKCS#7 bytes");

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
        assert_eq!(
            econtent_der.first(),
            Some(&0x30),
            "eContent must be a SEQUENCE"
        );

        // ASSERT
        assert!(econtent_der.get(1).expect("length octet") < &0x80);
        let hdr_len = 2;
        let value_octets = econtent_der.get(hdr_len..).expect("eContent value octets");

        let signer_info = sd.signer_infos.0.iter().next().expect("signer info");
        let signed_attrs = signer_info.signed_attrs.as_ref().expect("signed attrs");

        let md_oid = const_oid::ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
        let md_attr = signed_attrs
            .iter()
            .find(|attribute| attribute.oid == md_oid)
            .expect("messageDigest attribute must exist");
        let md_value = md_attr.values.iter().next().expect("md value");
        let md_octet = md_value
            .decode_as::<OctetString>()
            .expect("decode md as OctetString");
        let message_digest = md_octet.as_bytes();

        let computed = Sha256::digest(value_octets);
        let computed: &[u8] = computed.as_ref();

        assert_eq!(
            message_digest, computed,
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
        let mut buf = Vec::new();
        let result = sign(&mut &b"not-a-pe-file"[..], &signer, &cert, &mut buf);

        // ASSERT
        result.expect_err("invalid PE should fail");
    }

    #[test]
    fn push_u8_rejects_values_larger_than_byte() {
        // ARRANGE
        let mut encoded = Vec::new();

        // ACT
        let result = pkcs7::push_u8(&mut encoded, 0x100);

        // ASSERT
        result.expect_err("large byte value should fail");
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
            usize::try_from(read_u32_le(&cert, 0).expect("read length"))
                .expect("certificate length fits usize"),
            cert.len()
        );
        assert_eq!(read_u16_le(&cert, 4).expect("read revision"), 0x0200);
        assert_eq!(read_u16_le(&cert, 6).expect("read type"), 0x0002);
        assert_eq!(cert.get(8..).expect("PKCS#7 bytes"), pkcs7);
    }

    #[test]
    fn helper_alignment_and_write_success_paths_work() {
        // ARRANGE
        let mut data = [0_u8; 8];

        // ACT
        let aligned = align::up(9, 8).expect("align length");
        put_u32_le(&mut data, 2, 0x1234_5678).expect("write u32");

        // ASSERT
        assert_eq!(aligned, 16);
        assert_eq!(
            data.get(2..6).expect("u32 bytes"),
            &0x1234_5678_u32.to_le_bytes()
        );
    }

    #[test]
    fn align_to_rejects_adjustment_overflow() {
        // ARRANGE & ACT
        let result = align::up(usize::MAX, 8);
        let zero_result = align::up(0, 8);

        // ASSERT
        result.expect_err("alignment overflow should fail");
        assert_eq!(zero_result.expect("align zero"), 0);
    }

    #[test]
    fn checksum_and_bounds_helpers_validate_inputs() {
        // ARRANGE & ACT
        let too_large = usize::try_from(u32::MAX)
            .expect("u32 max fits usize")
            .checked_add(1)
            .expect("oversized usize");
        let conversion_result: core::result::Result<u32, _> = u32::try_from(too_large);
        let offset_result: core::result::Result<usize, _> = usize::try_from(0_u32);
        let add_result = usize::MAX.checked_add(1);
        let align_result = align::up(5, 0);

        // ASSERT
        conversion_result.expect_err("large usize conversion should fail");
        assert_eq!(offset_result.expect("convert offset"), 0);
        assert!(add_result.is_none(), "addition overflow should be None");
        align_result.expect_err("zero alignment should fail");
    }
}
