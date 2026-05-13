//! PEM key parsing for ECDSA P-256 signing and verification.

use base64ct::Encoding;
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair};

use crate::error::{KociError, Result};

/// Parse a PKCS#8 PEM-encoded ECDSA P-256 private key.
pub(crate) fn parse_pem_private_key(pem: &str) -> Result<EcdsaKeyPair> {
    let mut b64 = String::new();
    let mut in_block = false;

    for line in pem.lines() {
        let line = line.trim();
        if line == "-----BEGIN PRIVATE KEY-----" {
            in_block = true;
            continue;
        }
        if line == "-----END PRIVATE KEY-----" {
            break;
        }
        if in_block {
            b64.push_str(line);
        }
    }

    if b64.is_empty() {
        return Err(KociError::SignatureVerificationFailed(
            "No private key data found in PEM (expected PKCS#8 '-----BEGIN PRIVATE KEY-----')"
                .to_string(),
        ));
    }

    let der = base64ct::Base64::decode_vec(&b64).map_err(|e| {
        KociError::SignatureVerificationFailed(format!(
            "Failed to decode private key from PEM: {}",
            e
        ))
    })?;

    EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &der, &SystemRandom::new()).map_err(
        |_| {
            KociError::SignatureVerificationFailed(
                "Failed to parse ECDSA P-256 private key (must be PKCS#8 format)".to_string(),
            )
        },
    )
}

/// Parse a PEM-encoded ECDSA P-256 public key and return the raw SubjectPublicKeyInfo DER bytes.
pub(crate) fn parse_pem_public_key(pem: &str) -> Result<Vec<u8>> {
    let mut b64 = String::new();
    let mut in_block = false;

    for line in pem.lines() {
        let line = line.trim();
        if line == "-----BEGIN PUBLIC KEY-----" {
            in_block = true;
            continue;
        }
        if line == "-----END PUBLIC KEY-----" {
            break;
        }
        if in_block {
            b64.push_str(line);
        }
    }

    if b64.is_empty() {
        return Err(KociError::SignatureVerificationFailed(
            "No public key data found in PEM".to_string(),
        ));
    }

    let spki = base64ct::Base64::decode_vec(&b64).map_err(|e| {
        KociError::SignatureVerificationFailed(format!(
            "Failed to decode public key from PEM: {}",
            e
        ))
    })?;

    const POINT_OFFSET: usize = 26;
    const POINT_LEN: usize = 65;

    if spki.len() < POINT_OFFSET + POINT_LEN {
        return Err(KociError::SignatureVerificationFailed(
            "Public key SPKI DER is too short to contain a P-256 point".to_string(),
        ));
    }

    if spki[POINT_OFFSET] != 0x04 {
        return Err(KociError::SignatureVerificationFailed(
            "Public key is not an uncompressed EC point (expected 0x04 prefix)".to_string(),
        ));
    }

    Ok(spki[POINT_OFFSET..POINT_OFFSET + POINT_LEN].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn must<T, E: std::fmt::Display>(result: std::result::Result<T, E>, context: &str) -> T {
        match result {
            Ok(value) => value,
            Err(error) => panic!("{context}: {error}"),
        }
    }

    #[test]
    fn parse_pem_valid() {
        // ARRANGE
        let pem = "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVDS8kndtUxfYwqGcX2Dw2spTvR44\nt/4lr1W4h75GrFa0zqJwfH9v9oLH5Er0joEKk29+Dya7ZHXDGRiDGoJeYw==\n-----END PUBLIC KEY-----\n";

        // ACT
        let bytes = must(parse_pem_public_key(pem), "parse PEM public key");

        // ASSERT
        assert_eq!(bytes.len(), 65, "expected 65-byte uncompressed point");
        assert_eq!(bytes[0], 0x04, "expected uncompressed point prefix 0x04");
    }

    #[test]
    fn parse_pem_empty() {
        // ARRANGE
        let pem = "-----BEGIN PUBLIC KEY-----\n-----END PUBLIC KEY-----\n";

        // ACT & ASSERT
        assert!(parse_pem_public_key(pem).is_err());
    }

    #[test]
    fn parse_pem_no_markers() {
        // ARRANGE
        let input = "not a pem file";

        // ACT
        let result = parse_pem_public_key(input);

        // ASSERT
        assert!(result.is_err());
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
        let error = match parse_pem_public_key(pem) {
            Ok(_) => panic!("public key unexpectedly parsed"),
            Err(error) => error,
        };

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }

    #[test]
    fn parse_pem_public_key_rejects_compressed_point() {
        // ARRANGE
        let mut spki = vec![0u8; 26 + 65];
        spki[26] = 0x02;
        let pem = format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
            base64ct::Base64::encode_string(&spki)
        );

        // ACT
        let error = match parse_pem_public_key(&pem) {
            Ok(_) => panic!("public key unexpectedly parsed"),
            Err(error) => error,
        };

        // ASSERT
        assert!(matches!(error, KociError::SignatureVerificationFailed(_)));
    }
}
