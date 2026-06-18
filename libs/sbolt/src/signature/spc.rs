//! SPC indirect data content builder.

use const_oid::ObjectIdentifier;
use der::Encode as _;
use der::asn1::{Any, OctetString};
use spki::AlgorithmIdentifierOwned;

use crate::error::{Result, SboltError};
use crate::pkcs7::{
    self, DER_ONE_BYTE_LENGTH_LIMIT, DER_SHORT_FORM_LENGTH_LIMIT, DER_TWO_BYTE_LENGTH_LIMIT,
};

const SPC_PE_IMAGE_DATA_OBJID: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.3.6.1.4.1.311.2.1.15");
const SHA256_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");

/// Build the inner fields of `SpcIndirectDataContent`.
pub(super) fn build_spc_indirect_data(hash: &[u8; 32]) -> Result<Vec<u8>> {
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

    let obsolete_bmp: [u8; 28] = [
        0x00, 0x3c, 0x00, 0x3c, 0x00, 0x3c, 0x00, 0x4f, 0x00, 0x62, 0x00, 0x73, 0x00, 0x6f, 0x00,
        0x6c, 0x00, 0x65, 0x00, 0x74, 0x00, 0x65, 0x00, 0x3e, 0x00, 0x3e, 0x00, 0x3e,
    ];

    let mut unicode_field = Vec::new();
    unicode_field.push(implicit_primitive_tag);
    pkcs7::push_u8(&mut unicode_field, obsolete_bmp.len())?;
    unicode_field.extend_from_slice(&obsolete_bmp);

    let mut file_choice = Vec::new();
    file_choice.push(constructed_2_tag);
    pkcs7::push_u8(&mut file_choice, unicode_field.len())?;
    file_choice.extend_from_slice(&unicode_field);

    let mut spc_link = Vec::new();
    spc_link.push(constructed_tag);
    pkcs7::push_u8(&mut spc_link, file_choice.len())?;
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
pub(super) fn encode_length(buf: &mut Vec<u8>, len: usize) -> Result<()> {
    if len < DER_SHORT_FORM_LENGTH_LIMIT {
        pkcs7::push_u8(buf, len & 0xff)?;
    } else if len < DER_ONE_BYTE_LENGTH_LIMIT {
        buf.push(0x81);
        pkcs7::push_u8(buf, len & 0xff)?;
    } else if len < DER_TWO_BYTE_LENGTH_LIMIT {
        buf.push(0x82);
        pkcs7::push_u8(buf, len >> 8)?;
        pkcs7::push_u8(buf, len & 0xff)?;
    } else {
        buf.push(0x83);
        pkcs7::push_u8(buf, len >> 16)?;
        pkcs7::push_u8(buf, (len >> 8) & 0xff)?;
        pkcs7::push_u8(buf, len & 0xff)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        result.expect_err("four-byte length should fail");
    }
}
