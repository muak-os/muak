//! LUKS2 volume key digest creation and verification.

use base64ct::{Base64, Encoding};
use ring::pbkdf2;
use ring::rand::SecureRandom;

use crate::constants::DIGEST_ITERATIONS;
use crate::error::{Error, Result};
use crate::metadata::Digest;

const SHA256_LEN: usize = 32;
const SALT_LEN: usize = 32;

static PBKDF2_ALG: pbkdf2::Algorithm = pbkdf2::PBKDF2_HMAC_SHA256;

/// Creates a new PBKDF2-SHA256 digest of the volume key.
pub fn create(volume_key: &[u8], keyslot_ids: &[&str], segment_ids: &[&str]) -> Result<Digest> {
    let rng = ring::rand::SystemRandom::new();
    let mut salt = [0u8; SALT_LEN];
    rng.fill(&mut salt)
        .map_err(|_| Error::InvalidField("random generation failed".into()))?;

    let mut digest_value = [0u8; SHA256_LEN];
    pbkdf2::derive(
        PBKDF2_ALG,
        std::num::NonZeroU32::new(DIGEST_ITERATIONS)
            .ok_or_else(|| Error::InvalidField("invalid iteration count".into()))?,
        &salt,
        volume_key,
        &mut digest_value,
    );

    Ok(Digest {
        r#type: "pbkdf2".to_string(),
        keyslots: keyslot_ids.iter().map(|s| s.to_string()).collect(),
        segments: segment_ids.iter().map(|s| s.to_string()).collect(),
        hash: "sha256".to_string(),
        iterations: DIGEST_ITERATIONS,
        salt: Base64::encode_string(&salt),
        digest: Base64::encode_string(&digest_value),
    })
}

/// Verifies a volume key candidate against a stored digest.
pub fn verify(volume_key: &[u8], digest: &Digest) -> Result<bool> {
    if digest.r#type != "pbkdf2" {
        return Err(Error::InvalidField(format!(
            "unsupported digest type: {}",
            digest.r#type
        )));
    }

    let salt = Base64::decode_vec(&digest.salt)?;
    let expected = Base64::decode_vec(&digest.digest)?;

    let iterations = std::num::NonZeroU32::new(digest.iterations)
        .ok_or_else(|| Error::InvalidField("invalid iteration count".into()))?;

    let result = pbkdf2::verify(PBKDF2_ALG, iterations, &salt, volume_key, &expected);

    Ok(result.is_ok())
}

#[cfg(test)]
mod tests {
    use base64ct::Encoding;

    use super::*;
    use crate::metadata::Digest;

    #[test]
    fn test_create_verify_roundtrip() {
        // ARRANGE
        let volume_key = vec![0x42u8; 64];
        let keyslot_ids = &["0"];
        let segment_ids = &["0"];

        // ACT
        let digest = create(&volume_key, keyslot_ids, segment_ids).unwrap();

        // ASSERT
        let result = verify(&volume_key, &digest).unwrap();
        assert!(result);
    }

    #[test]
    fn test_verify_wrong_key() {
        // ARRANGE
        let volume_key = vec![0x42u8; 64];
        let wrong_key = vec![0x43u8; 64];
        let keyslot_ids = &["0"];
        let segment_ids = &["0"];

        let digest = create(&volume_key, keyslot_ids, segment_ids).unwrap();

        // ACT
        let result = verify(&wrong_key, &digest).unwrap();

        // ASSERT
        assert!(!result);
    }

    #[test]
    fn test_verify_modified_key() {
        // ARRANGE
        let volume_key = vec![0x42u8; 64];
        let keyslot_ids = &["0"];
        let segment_ids = &["0"];

        let digest = create(&volume_key, keyslot_ids, segment_ids).unwrap();

        let mut modified_key = volume_key.clone();
        modified_key[0] ^= 0x01;

        // ACT
        let result = verify(&modified_key, &digest).unwrap();

        // ASSERT
        assert!(!result);
    }

    #[test]
    fn test_create_different_salts() {
        // ARRANGE
        let volume_key = vec![0x42u8; 64];
        let keyslot_ids = &["0"];
        let segment_ids = &["0"];

        // ACT
        let digest1 = create(&volume_key, keyslot_ids, segment_ids).unwrap();
        let digest2 = create(&volume_key, keyslot_ids, segment_ids).unwrap();

        // ASSERT
        assert_ne!(digest1.salt, digest2.salt);

        assert!(verify(&volume_key, &digest1).unwrap());
        assert!(verify(&volume_key, &digest2).unwrap());
    }

    #[test]
    fn test_digest_structure() {
        // ARRANGE
        let volume_key = vec![0x42u8; 64];
        let keyslot_ids = &["0", "1"];
        let segment_ids = &["0", "1", "2"];

        // ACT
        let digest = create(&volume_key, keyslot_ids, segment_ids).unwrap();

        // ASSERT
        assert_eq!(digest.r#type, "pbkdf2");
        assert_eq!(digest.hash, "sha256");
        assert_eq!(digest.iterations, DIGEST_ITERATIONS);
        assert_eq!(digest.keyslots, vec!["0".to_string(), "1".to_string()]);
        assert_eq!(
            digest.segments,
            vec!["0".to_string(), "1".to_string(), "2".to_string()]
        );

        assert!(!digest.salt.is_empty());
        assert!(!digest.digest.is_empty());
    }

    #[test]
    fn test_verify_unsupported_digest_type() {
        // ARRANGE
        let volume_key = vec![0x42u8; 64];
        let digest = Digest {
            r#type: "argon2".to_string(),
            keyslots: vec!["0".to_string()],
            segments: vec!["0".to_string()],
            hash: "sha256".to_string(),
            iterations: 1000,
            salt: base64ct::Base64::encode_string(&[0x42u8; 32]),
            digest: base64ct::Base64::encode_string(&[0x42u8; 32]),
        };

        // ACT
        let result = verify(&volume_key, &digest);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_zero_iterations() {
        // ARRANGE
        let volume_key = vec![0x42u8; 64];
        let digest = Digest {
            r#type: "pbkdf2".to_string(),
            keyslots: vec!["0".to_string()],
            segments: vec!["0".to_string()],
            hash: "sha256".to_string(),
            iterations: 0,
            salt: base64ct::Base64::encode_string(&[0x42u8; 32]),
            digest: base64ct::Base64::encode_string(&[0x42u8; 32]),
        };

        // ACT
        let result = verify(&volume_key, &digest);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_empty_key() {
        // ARRANGE
        let volume_key = vec![];
        let keyslot_ids = &["0"];
        let segment_ids = &["0"];

        let digest = create(&volume_key, keyslot_ids, segment_ids).unwrap();

        // ACT
        let result = verify(&volume_key, &digest).unwrap();

        // ASSERT
        assert!(result);
    }

    #[test]
    fn test_verify_different_key_sizes() {
        // ARRANGE & ACT & ASSERT
        for size in [16, 32, 64, 128] {
            let volume_key = vec![0x42u8; size];
            let keyslot_ids = &["0"];
            let segment_ids = &["0"];

            let digest = create(&volume_key, keyslot_ids, segment_ids).unwrap();

            let result = verify(&volume_key, &digest).unwrap();
            assert!(result, "Failed for key size {}", size);
        }
    }

    #[test]
    fn test_verify_corrupted_digest() {
        // ARRANGE
        let volume_key = vec![0x42u8; 64];
        let keyslot_ids = &["0"];
        let segment_ids = &["0"];

        let mut digest = create(&volume_key, keyslot_ids, segment_ids).unwrap();

        let mut decoded = base64ct::Base64::decode_vec(&digest.digest).unwrap();
        decoded[0] ^= 0xFF;
        digest.digest = base64ct::Base64::encode_string(&decoded);

        // ACT
        let result = verify(&volume_key, &digest).unwrap();

        // ASSERT
        assert!(!result);
    }

    #[test]
    fn test_verify_corrupted_salt() {
        // ARRANGE
        let volume_key = vec![0x42u8; 64];
        let keyslot_ids = &["0"];
        let segment_ids = &["0"];

        let mut digest = create(&volume_key, keyslot_ids, segment_ids).unwrap();

        let mut decoded = base64ct::Base64::decode_vec(&digest.salt).unwrap();
        decoded[0] ^= 0xFF;
        digest.salt = base64ct::Base64::encode_string(&decoded);

        // ACT
        let result = verify(&volume_key, &digest).unwrap();

        // ASSERT
        assert!(!result);
    }
}
