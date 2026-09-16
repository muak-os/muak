//! Cryptographic digest utilities for OCI blob integrity.

use sha2::{Digest as _, Sha256};

use crate::error::{OciError, Result};

/// Streaming SHA-256 digest verifier.
pub struct Verifier {
    context: Sha256,
    expected: String,
}

impl Verifier {
    /// Create a new streaming digest verifier for the given OCI digest.
    ///
    /// # Errors
    ///
    /// Returns an error when the digest does not use the `sha256:` algorithm.
    pub fn new(expected_digest: &str) -> Result<Self> {
        let expected_hash =
            expected_digest
                .strip_prefix("sha256:")
                .ok_or_else(|| OciError::DigestMismatch {
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
    pub fn update(&mut self, chunk: &[u8]) {
        self.context.update(chunk);
    }

    /// Finalize and verify the digest matches the expected value.
    ///
    /// # Errors
    ///
    /// Returns an error when the accumulated bytes do not match the digest.
    pub fn verify(self) -> Result<()> {
        let hash = self.context.finalize();
        let actual = base16ct::lower::encode_string(hash.as_ref());

        if actual != self.expected {
            return Err(OciError::DigestMismatch {
                resource: "blob".to_owned(),
                expected: format!("sha256:{}", self.expected),
                actual,
            });
        }

        Ok(())
    }
}

/// Compute the SHA-256 hex digest of the given bytes.
#[must_use]
pub fn sha256_hex(data: &[u8]) -> String {
    base16ct::lower::encode_string(Sha256::digest(data).as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_digest_verifies_hello() {
        // ARRANGE
        let digest = "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        let mut verifier = Verifier::new(digest).expect("create verifier");

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
        let mut verifier = Verifier::new(digest).expect("create verifier");

        // ACT
        verifier.update(b"wrong");
        let result = verifier.verify();

        // ASSERT
        assert!(matches!(result, Err(OciError::DigestMismatch { .. })));
    }

    #[test]
    fn streaming_digest_rejects_unsupported_algorithm() {
        // ACT
        let result = Verifier::new("md5:abcdef");

        // ASSERT
        assert!(matches!(result, Err(OciError::DigestMismatch { .. })));
    }
}
