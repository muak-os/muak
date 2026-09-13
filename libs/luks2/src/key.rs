//! LUKS key material.

use base64ct::{Base64Unpadded, Encoding as _};
use zeroize::Zeroizing;

use crate::error::{Luks2Error, Result};

/// Length in bytes of a generated LUKS key.
const GENERATE_LEN: usize = 64;

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
///
/// Returns `None` when the encoded key is malformed.
#[must_use]
pub fn from(encoded: &str) -> Option<Passphrase> {
    Base64Unpadded::decode_vec(encoded).ok().map(Zeroizing::new)
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
