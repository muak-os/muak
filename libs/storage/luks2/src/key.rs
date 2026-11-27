//! LUKS key material.

use base64ct::{Base64Unpadded, Encoding as _};
use zeroize::Zeroizing;

use crate::error::{Luks2Error, Result};

/// Length in bytes of a generated LUKS key.
const GENERATE_LEN: usize = 64;

/// Length in bytes of a LUKS2 volume key.
pub const VOLUME_KEY_SIZE: usize = 64;

/// Length in bytes of a LUKS2 volume key, typed for JSON `key_size` fields.
pub(crate) const VOLUME_KEY_SIZE_U32: u32 = 64;

/// Zeroizing LUKS key bytes.
pub type Passphrase = Zeroizing<Vec<u8>>;

/// Generates a random LUKS key.
///
/// # Errors
///
/// Returns an error if the system random source fails.
pub fn generate() -> Result<Passphrase> {
    let mut key = vec![0_u8; GENERATE_LEN];
    getrandom::fill(&mut key).map_err(|_error| Luks2Error::Rng)?;

    Ok(Zeroizing::new(key))
}

/// Decodes a LUKS key from its unpadded Base64 transport encoding.
#[must_use = "callers must handle a malformed key"]
pub fn from(encoded: &str) -> Option<Passphrase> {
    Base64Unpadded::decode_vec(encoded).ok().map(Zeroizing::new)
}

/// Wraps raw key bytes, validating them against the volume key size.
///
/// # Errors
///
/// Returns [`Luks2Error::InvalidKeyLength`] when `bytes` is not
/// [`VOLUME_KEY_SIZE`] long. The rejected buffer is zeroized on drop.
pub fn from_raw(bytes: Vec<u8>) -> Result<Passphrase> {
    let key = Passphrase::new(bytes);

    if key.len() != VOLUME_KEY_SIZE {
        return Err(Luks2Error::InvalidKeyLength {
            actual: key.len(),
            expected: VOLUME_KEY_SIZE,
        });
    }

    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_returns_key_of_expected_length() {
        // ARRANGE & ACT
        let key = generate().unwrap();

        // ASSERT
        assert_eq!(key.len(), GENERATE_LEN, "key length should match");
    }

    #[test]
    fn from_decodes_valid_key() {
        // ARRANGE
        let encoded = "c2VjcmV0LWtleS1kYXRh";

        // ACT
        let result = from(encoded);

        // ASSERT
        assert_eq!(
            result.as_ref().map(|value| value.as_slice()),
            Some(b"secret-key-data".as_ref())
        );
    }

    #[test]
    fn from_returns_none_on_invalid_encoding() {
        // ARRANGE
        let encoded = "!!!not-base64!!!";

        // ACT & ASSERT
        assert!(from(encoded).is_none());
    }

    #[test]
    fn from_raw_accepts_volume_key_size() {
        // ARRANGE
        let bytes = vec![0xAB; VOLUME_KEY_SIZE];

        // ACT
        let key = from_raw(bytes);

        // ASSERT
        assert_eq!(
            key.as_ref().map(|value| value.as_slice()).ok(),
            Some([0xAB; VOLUME_KEY_SIZE].as_ref())
        );
    }

    #[test]
    fn from_raw_rejects_empty_buffer() {
        // ARRANGE
        let bytes = Vec::new();

        // ACT
        let error = from_raw(bytes).unwrap_err();

        // ASSERT
        assert_eq!(
            error.to_string(),
            "invalid LUKS key length: 0 bytes, expected 64"
        );
    }

    #[test]
    fn from_raw_rejects_truncated_buffer() {
        // ARRANGE
        let bytes = vec![0xAB; VOLUME_KEY_SIZE - 1];

        // ACT
        let error = from_raw(bytes).unwrap_err();

        // ASSERT
        assert_eq!(
            error.to_string(),
            "invalid LUKS key length: 63 bytes, expected 64"
        );
    }

    #[test]
    fn from_raw_rejects_oversized_buffer() {
        // ARRANGE
        let bytes = vec![0xAB; VOLUME_KEY_SIZE + 1];

        // ACT
        let error = from_raw(bytes).unwrap_err();

        // ASSERT
        assert_eq!(
            error.to_string(),
            "invalid LUKS key length: 65 bytes, expected 64"
        );
    }

    #[test]
    fn from_handles_empty_value() {
        // ARRANGE
        let encoded = "";

        // ACT
        let result = from(encoded);

        // ASSERT
        assert_eq!(
            result.as_ref().map(|value| value.as_slice()),
            Some([].as_ref())
        );
    }
}
