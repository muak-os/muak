//! LUKS2 volume key digest creation and verification.

use core::num::NonZeroU32;

use base64ct::{Base64, Encoding as _};
use subtle::ConstantTimeEq as _;

use crate::error::{Luks2Error as Error, Result};
use crate::metadata::Digest;
use crate::pbkdf2;

const DIGEST_ITERATIONS: u32 = 1_000;
const SALT_LEN: usize = 32;

/// Creates a new PBKDF2-SHA256 digest of the volume key.
pub fn create(volume_key: &[u8], keyslot_ids: &[&str], segment_ids: &[&str]) -> Result<Digest> {
    let mut salt = [0_u8; SALT_LEN];
    getrandom::fill(&mut salt)
        .map_err(|_error| Error::InvalidField("random generation failed".into()))?;

    let iterations = NonZeroU32::new(DIGEST_ITERATIONS)
        .ok_or_else(|| Error::InvalidField("invalid iteration count".into()))?;
    let digest_value = pbkdf2::derive(volume_key, &salt, iterations)?;

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

    let computed = pbkdf2::derive(volume_key, &salt, iterations)?;

    Ok(computed.as_slice().ct_eq(&expected).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_verify_roundtrip() {
        // ARRANGE
        let volume_key = vec![0x42_u8; 64];

        // ACT
        let digest = create(&volume_key, &["ks0"], &["seg0"]).expect("create digest");
        let verified = verify(&volume_key, &digest).expect("verify digest");

        // ASSERT
        assert!(verified);
    }

    #[test]
    fn verify_rejects_wrong_volume_key() {
        // ARRANGE
        let volume_key = vec![0x42_u8; 64];
        let digest = create(&volume_key, &["ks0"], &["seg0"]).expect("create digest");
        let wrong_key = vec![0x43_u8; 64];

        // ACT
        let verified = verify(&wrong_key, &digest).expect("verify digest");

        // ASSERT
        assert!(!verified);
    }

    #[test]
    fn verify_rejects_unsupported_digest_type() {
        // ARRANGE
        let volume_key = vec![0x42_u8; 64];
        let mut digest = create(&volume_key, &["ks0"], &["seg0"]).expect("create digest");
        digest.r#type = "argon2id".to_owned();

        // ACT
        let result = verify(&volume_key, &digest);

        // ASSERT
        result.expect_err("unsupported digest type should fail");
    }

    #[test]
    fn create_uses_pbkdf2_sha256_fields() {
        // ARRANGE
        let volume_key = vec![0x42_u8; 64];

        // ACT
        let digest = create(&volume_key, &["ks0"], &["seg0"]).expect("create digest");

        // ASSERT
        assert_eq!(digest.r#type, "pbkdf2");
        assert_eq!(digest.hash, "sha256");
        assert_eq!(digest.iterations, DIGEST_ITERATIONS);
        assert_eq!(digest.keyslots, vec!["ks0".to_owned()]);
        assert_eq!(digest.segments, vec!["seg0".to_owned()]);
    }
}
