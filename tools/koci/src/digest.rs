//! Cryptographic digest utilities for OCI blob integrity.

use crate::error::{KociError, Result};

/// Compute the SHA-256 hex digest of the given bytes.
pub(crate) fn sha256_hex(data: &[u8]) -> String {
    use ring::digest;
    let hash = digest::digest(&digest::SHA256, data);
    hex_encode(hash.as_ref())
}

/// Encode bytes as a lowercase hex string.
pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

/// Verify that the SHA-256 digest of a downloaded blob matches its expected OCI digest.
pub(crate) fn verify_blob_digest(data: &[u8], expected_digest: &str) -> Result<()> {
    let expected_hash =
        expected_digest
            .strip_prefix("sha256:")
            .ok_or_else(|| KociError::DigestMismatch {
                resource: "blob".to_string(),
                expected: expected_digest.to_string(),
                actual: "unsupported digest algorithm".to_string(),
            })?;

    let actual_hash = sha256_hex(data);

    if actual_hash != expected_hash {
        return Err(KociError::DigestMismatch {
            resource: expected_digest.to_string(),
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_empty() {
        // ACT & ASSERT
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_hex_hello() {
        // ACT & ASSERT
        assert_eq!(
            sha256_hex(b"hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn verify_blob_digest_ok() {
        // ARRANGE
        let data = b"hello";
        let digest = "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

        // ACT
        let result = verify_blob_digest(data, digest);

        // ASSERT
        assert!(result.is_ok());
    }

    #[test]
    fn verify_blob_digest_unsupported_algorithm() {
        // ARRANGE
        let data = b"hello";
        let digest = "md5:abcdef";

        // ACT
        let result = verify_blob_digest(data, digest);

        // ASSERT
        assert!(matches!(result, Err(KociError::DigestMismatch { .. })));
    }
}
