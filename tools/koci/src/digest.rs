//! Cryptographic digest utilities for OCI blob integrity.

use sha2::{Digest as _, Sha256};

use crate::error::{KociError, Result};

/// Streaming SHA-256 digest verifier.
pub(crate) struct StreamingDigest {
    context: Sha256,
    expected: String,
}

impl StreamingDigest {
    /// Create a new streaming digest verifier for the given OCI digest.
    pub(crate) fn new(expected_digest: &str) -> Result<Self> {
        let expected_hash =
            expected_digest
                .strip_prefix("sha256:")
                .ok_or_else(|| KociError::DigestMismatch {
                    resource: "blob".to_owned(),
                    expected: expected_digest.to_owned(),
                    actual: "unsupported digest algorithm".to_owned(),
                })?;

        Ok(Self {
            context: Sha256::new(),
            expected: expected_hash.to_owned(),
        })
    }

    /// Feed a chunk of data into the digest.
    pub(crate) fn update(&mut self, chunk: &[u8]) {
        self.context.update(chunk);
    }

    /// Finalize and verify the digest matches the expected value.
    pub(crate) fn verify(self) -> Result<()> {
        let hash = self.context.finalize();
        let actual = hex_encode(hash.as_ref());

        if actual != self.expected {
            return Err(KociError::DigestMismatch {
                resource: "blob".to_owned(),
                expected: format!("sha256:{}", self.expected),
                actual,
            });
        }

        Ok(())
    }
}

/// Compute the SHA-256 hex digest of the given bytes.
pub(crate) fn sha256_hex(data: &[u8]) -> String {
    hex_encode(Sha256::digest(data).as_ref())
}

/// Encode bytes as a lowercase hex string.
pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    let mut encoded = String::new();
    for &byte in bytes {
        encoded.push(hex_digit(byte >> 4));
        encoded.push(hex_digit(byte & 0x0f));
    }
    encoded
}

fn hex_digit(nibble: u8) -> char {
    char::from_digit(u32::from(nibble), 16).unwrap_or('0')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_digest_verifies_hello() {
        // ARRANGE
        let digest = "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        let mut verifier = StreamingDigest::new(digest).expect("create verifier");

        // ACT
        verifier.update(b"hello");
        let result = verifier.verify();

        // ASSERT
        result.expect("digest should verify");
    }

    #[test]
    fn streaming_digest_detects_mismatch() {
        // ARRANGE
        let digest = "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        let mut verifier = StreamingDigest::new(digest).expect("create verifier");

        // ACT
        verifier.update(b"wrong");
        let result = verifier.verify();

        // ASSERT
        assert!(matches!(result, Err(KociError::DigestMismatch { .. })));
    }

    #[test]
    fn streaming_digest_rejects_unsupported_algorithm() {
        // ACT
        let result = StreamingDigest::new("md5:abcdef");

        // ASSERT
        assert!(matches!(result, Err(KociError::DigestMismatch { .. })));
    }
}
