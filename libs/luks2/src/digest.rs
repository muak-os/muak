//! LUKS2 volume key digest creation and verification.

use core::num::NonZeroU32;

use base64ct::{Base64, Encoding as _};
use ring::pbkdf2;
use ring::rand::{SecureRandom as _, SystemRandom};

use crate::error::{Luks2Error as Error, Result};
use crate::metadata::Digest;

const DIGEST_ITERATIONS: u32 = 1_000;
const SHA256_LEN: usize = 32;
const SALT_LEN: usize = 32;

static PBKDF2_ALG: pbkdf2::Algorithm = pbkdf2::PBKDF2_HMAC_SHA256;

/// Creates a new PBKDF2-SHA256 digest of the volume key.
pub fn create(volume_key: &[u8], keyslot_ids: &[&str], segment_ids: &[&str]) -> Result<Digest> {
    let rng = SystemRandom::new();
    let mut salt = [0_u8; SALT_LEN];
    rng.fill(&mut salt)
        .map_err(|_error| Error::InvalidField("random generation failed".into()))?;

    let mut digest_value = [0_u8; SHA256_LEN];
    pbkdf2::derive(
        PBKDF2_ALG,
        NonZeroU32::new(DIGEST_ITERATIONS)
            .ok_or_else(|| Error::InvalidField("invalid iteration count".into()))?,
        &salt,
        volume_key,
        &mut digest_value,
    );

    Ok(Digest {
        r#type: "pbkdf2".to_owned(),
        keyslots: keyslot_ids
            .iter()
            .map(|keyslot_id| (*keyslot_id).to_owned())
            .collect(),
        segments: segment_ids
            .iter()
            .map(|segment_id| (*segment_id).to_owned())
            .collect(),
        hash: "sha256".to_owned(),
        iterations: DIGEST_ITERATIONS,
        salt: Base64::encode_string(&salt),
        value: Base64::encode_string(&digest_value),
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
    let expected = Base64::decode_vec(&digest.value)?;

    let iterations = NonZeroU32::new(digest.iterations)
        .ok_or_else(|| Error::InvalidField("invalid iteration count".into()))?;

    let result = pbkdf2::verify(PBKDF2_ALG, iterations, &salt, volume_key, &expected);

    Ok(result.is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::Digest;

    #[test]
    fn create_verify_roundtrip() {
        // ARRANGE
        let volume_key = vec![0x42_u8; 64];
        let keyslot_ids = &["0"];
        let segment_ids = &["0"];

        // ACT
        let digest = create(&volume_key, keyslot_ids, segment_ids).unwrap();

        // ASSERT
        let result = verify(&volume_key, &digest).unwrap();
        assert!(result);
    }

    #[test]
    fn verify_wrong_key() {
        // ARRANGE
        let volume_key = vec![0x42_u8; 64];
        let wrong_key = vec![0x43_u8; 64];
        let keyslot_ids = &["0"];
        let segment_ids = &["0"];

        let digest = create(&volume_key, keyslot_ids, segment_ids).unwrap();

        // ACT
        let result = verify(&wrong_key, &digest).unwrap();

        // ASSERT
        assert!(!result);
    }

    #[test]
    fn verify_modified_key() {
        // ARRANGE
        let volume_key = vec![0x42_u8; 64];
        let keyslot_ids = &["0"];
        let segment_ids = &["0"];

        let digest = create(&volume_key, keyslot_ids, segment_ids).unwrap();

        let mut modified_key = volume_key.clone();
        *modified_key.get_mut(0).unwrap() ^= 0x01;

        // ACT
        let result = verify(&modified_key, &digest).unwrap();

        // ASSERT
        assert!(!result);
    }

    #[test]
    fn create_different_salts() {
        // ARRANGE
        let volume_key = vec![0x42_u8; 64];
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
    fn digest_structure() {
        // ARRANGE
        let volume_key = vec![0x42_u8; 64];
        let keyslot_ids = &["0", "1"];
        let segment_ids = &["0", "1", "2"];

        // ACT
        let digest = create(&volume_key, keyslot_ids, segment_ids).unwrap();

        // ASSERT
        assert_eq!(digest.r#type, "pbkdf2");
        assert_eq!(digest.hash, "sha256");
        assert_eq!(digest.iterations, DIGEST_ITERATIONS);
        assert_eq!(digest.keyslots, vec!["0".to_owned(), "1".to_owned()]);
        assert_eq!(
            digest.segments,
            vec!["0".to_owned(), "1".to_owned(), "2".to_owned()]
        );

        assert!(!digest.salt.is_empty());
        assert!(!digest.value.is_empty());
    }

    #[test]
    fn verify_unsupported_digest_type() {
        // ARRANGE
        let volume_key = vec![0x42_u8; 64];
        let digest = Digest {
            r#type: "argon2".to_owned(),
            keyslots: vec!["0".to_owned()],
            segments: vec!["0".to_owned()],
            hash: "sha256".to_owned(),
            iterations: 1000,
            salt: base64ct::Base64::encode_string(&[0x42_u8; 32]),
            value: base64ct::Base64::encode_string(&[0x42_u8; 32]),
        };

        // ACT
        let result = verify(&volume_key, &digest);

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn verify_zero_iterations() {
        // ARRANGE
        let volume_key = vec![0x42_u8; 64];
        let digest = Digest {
            r#type: "pbkdf2".to_owned(),
            keyslots: vec!["0".to_owned()],
            segments: vec!["0".to_owned()],
            hash: "sha256".to_owned(),
            iterations: 0,
            salt: base64ct::Base64::encode_string(&[0x42_u8; 32]),
            value: base64ct::Base64::encode_string(&[0x42_u8; 32]),
        };

        // ACT
        let result = verify(&volume_key, &digest);

        // ASSERT
        result.unwrap_err();
    }

    #[test]
    fn verify_empty_key() {
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
    fn verify_different_key_sizes() {
        // ARRANGE & ACT & ASSERT
        for size in [16, 32, 64, 128] {
            let volume_key = vec![0x42_u8; size];
            let keyslot_ids = &["0"];
            let segment_ids = &["0"];

            let digest = create(&volume_key, keyslot_ids, segment_ids).unwrap();

            let result = verify(&volume_key, &digest).unwrap();
            assert!(result, "Failed for key size {size}");
        }
    }

    #[test]
    fn verify_corrupted_digest() {
        // ARRANGE
        let volume_key = vec![0x42_u8; 64];
        let keyslot_ids = &["0"];
        let segment_ids = &["0"];

        let mut digest = create(&volume_key, keyslot_ids, segment_ids).unwrap();

        let mut decoded = base64ct::Base64::decode_vec(&digest.value).unwrap();
        *decoded.get_mut(0).unwrap() ^= 0xFF;
        digest.value = base64ct::Base64::encode_string(&decoded);

        // ACT
        let result = verify(&volume_key, &digest).unwrap();

        // ASSERT
        assert!(!result);
    }

    #[test]
    fn verify_corrupted_salt() {
        // ARRANGE
        let volume_key = vec![0x42_u8; 64];
        let keyslot_ids = &["0"];
        let segment_ids = &["0"];

        let mut digest = create(&volume_key, keyslot_ids, segment_ids).unwrap();

        let mut decoded = base64ct::Base64::decode_vec(&digest.salt).unwrap();
        *decoded.get_mut(0).unwrap() ^= 0xFF;
        digest.salt = base64ct::Base64::encode_string(&decoded);

        // ACT
        let result = verify(&volume_key, &digest).unwrap();

        // ASSERT
        assert!(!result);
    }

    #[test]
    fn verify_invalid_base64_digest_returns_error() {
        // ARRANGE
        let digest = Digest {
            r#type: String::from("pbkdf2"),
            keyslots: vec![String::from("0")],
            segments: vec![String::from("0")],
            hash: String::from("sha256"),
            iterations: 1,
            salt: String::from("%%%"),
            value: String::from("%%%"),
        };

        // ACT
        let result = verify(b"key", &digest);

        // ASSERT
        result.unwrap_err();
    }
}
