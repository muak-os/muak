//! PEM key parsing for ECDSA P-256 signing and verification.

use base64ct::Encoding as _;
use p256::ecdsa::SigningKey;
use p256::elliptic_curve::pkcs8::DecodePrivateKey as _;

use crate::error::{KociError, Result};

const POINT_OFFSET: usize = 26;
const POINT_LEN: usize = 65;

/// Parse a PKCS#8 PEM-encoded ECDSA P-256 private key.
pub(crate) fn parse_pem_private_key(pem: &str) -> Result<SigningKey> {
    let b64 = pem_body(
        pem,
        "-----BEGIN PRIVATE KEY-----",
        "-----END PRIVATE KEY-----",
    );

    if b64.is_empty() {
        return Err(KociError::SignatureVerificationFailed(
            "No private key data found in PEM (expected PKCS#8 '-----BEGIN PRIVATE KEY-----')"
                .to_owned(),
        ));
    }

    let der = base64ct::Base64::decode_vec(&b64).map_err(|error| {
        KociError::SignatureVerificationFailed(format!(
            "Failed to decode private key from PEM: {error}"
        ))
    })?;

    SigningKey::from_pkcs8_der(&der).map_err(|error| {
        KociError::SignatureVerificationFailed(format!(
            "Failed to parse ECDSA P-256 private key (must be PKCS#8 format): {error}"
        ))
    })
}

/// Parse a PEM-encoded ECDSA P-256 public key and return the raw `SubjectPublicKeyInfo` DER bytes.
pub(crate) fn parse_pem_public_key(pem: &str) -> Result<Vec<u8>> {
    let b64 = pem_body(
        pem,
        "-----BEGIN PUBLIC KEY-----",
        "-----END PUBLIC KEY-----",
    );

    if b64.is_empty() {
        return Err(KociError::SignatureVerificationFailed(
            "No public key data found in PEM".to_owned(),
        ));
    }

    let spki = base64ct::Base64::decode_vec(&b64).map_err(|error| {
        KociError::SignatureVerificationFailed(format!(
            "Failed to decode public key from PEM: {error}"
        ))
    })?;

    if spki.len() < POINT_OFFSET + POINT_LEN {
        return Err(KociError::SignatureVerificationFailed(
            "Public key SPKI DER is too short to contain a P-256 point".to_owned(),
        ));
    }

    if spki.get(POINT_OFFSET) != Some(&0x04) {
        return Err(KociError::SignatureVerificationFailed(
            "Public key is not an uncompressed EC point (expected 0x04 prefix)".to_owned(),
        ));
    }

    Ok(spki
        .iter()
        .skip(POINT_OFFSET)
        .take(POINT_LEN)
        .copied()
        .collect())
}

fn pem_body(pem: &str, begin_marker: &str, end_marker: &str) -> String {
    let mut body = String::new();
    let mut in_block = false;

    for line in pem.lines() {
        let line = line.trim();
        if line == begin_marker {
            in_block = true;
            continue;
        }
        if line == end_marker {
            break;
        }
        if in_block {
            body.push_str(line);
        }
    }

    body
}

#[cfg(test)]
mod tests {
    use base64ct::Base64;
    use getrandom::SysRng;
    use p256::elliptic_curve::Generate as _;
    use p256::elliptic_curve::pkcs8::EncodePrivateKey as _;
    use p256::elliptic_curve::sec1::ToSec1Point as _;

    use super::*;

    fn generate_private_key_pem() -> String {
        let key = SigningKey::try_generate_from_rng(&mut SysRng).expect("generate private key");
        let pkcs8 = key.to_pkcs8_der().expect("encode private key");
        format!(
            "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
            Base64::encode_string(pkcs8.as_bytes())
        )
    }

    #[test]
    fn parse_pem_valid() {
        // ARRANGE
        let pem = "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVDS8kndtUxfYwqGcX2Dw2spTvR44\nt/4lr1W4h75GrFa0zqJwfH9v9oLH5Er0joEKk29+Dya7ZHXDGRiDGoJeYw==\n-----END PUBLIC KEY-----\n";

        // ACT
        let bytes = parse_pem_public_key(pem).expect("parse PEM public key");

        // ASSERT
        assert_eq!(bytes.len(), 65, "expected 65-byte uncompressed point");
        assert_eq!(
            bytes.first(),
            Some(&0x04),
            "expected uncompressed point prefix 0x04"
        );
    }

    #[test]
    fn parse_pem_empty() {
        // ARRANGE
        let pem = "-----BEGIN PUBLIC KEY-----\n-----END PUBLIC KEY-----\n";

        // ACT & ASSERT
        parse_pem_public_key(pem).expect_err("public key parsing should fail");
    }

    #[test]
    fn parse_pem_no_markers() {
        // ARRANGE
        let input = "not a pem file";

        // ACT
        let result = parse_pem_public_key(input);

        // ASSERT
        result.expect_err("public key parsing should fail");
    }

    #[test]
    fn parse_pem_private_key_rejects_invalid_base64() {
        // ARRANGE
        let pem = "-----BEGIN PRIVATE KEY-----\n!!!\n-----END PRIVATE KEY-----\n";

        // ACT / ASSERT
        let error = parse_pem_private_key(pem).expect_err("private key parsing should fail");
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }

    #[test]
    fn parse_pem_private_key_accepts_valid_pkcs8() {
        // ARRANGE
        let pem = generate_private_key_pem();

        // ACT
        let key_pair = parse_pem_private_key(&pem).expect("private key parsing should succeed");

        // ASSERT
        assert!(
            !key_pair
                .verifying_key()
                .as_affine()
                .to_sec1_point(false)
                .as_bytes()
                .is_empty()
        );
    }

    #[test]
    fn parse_pem_private_key_rejects_missing_pem_block() {
        // ARRANGE / ACT
        let error =
            parse_pem_private_key("not a pem file").expect_err("private key parsing should fail");

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }

    #[test]
    fn parse_pem_private_key_rejects_invalid_pkcs8_bytes() {
        // ARRANGE
        let pem = "-----BEGIN PRIVATE KEY-----\nAAECAwQFBgc=\n-----END PRIVATE KEY-----\n";

        // ACT / ASSERT
        let error = parse_pem_private_key(pem).expect_err("private key parsing should fail");
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }

    #[test]
    fn parse_pem_public_key_rejects_short_spki() {
        // ARRANGE
        let pem = "-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----\n";

        // ACT
        let error = parse_pem_public_key(pem).expect_err("public key parsing should fail");

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }

    #[test]
    fn parse_pem_public_key_rejects_compressed_point() {
        // ARRANGE
        let mut spki = vec![0_u8; 26 + 65];
        *spki
            .get_mut(26)
            .expect("SPKI test fixture has point prefix") = 0x02;
        let pem = format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
            base64ct::Base64::encode_string(&spki)
        );

        // ACT
        let error = parse_pem_public_key(&pem).expect_err("public key parsing should fail");

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }

    #[test]
    fn parse_pem_public_key_rejects_invalid_base64() {
        // ARRANGE
        let pem = "-----BEGIN PUBLIC KEY-----\n!!!\n-----END PUBLIC KEY-----\n";

        // ACT
        let error = parse_pem_public_key(pem).expect_err("public key parsing should fail");

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }
}
