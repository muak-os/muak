//! LUKS2 volume key digest creation and verification.
//!
//! Uses PBKDF2-HMAC-SHA256 via `ring` to create and verify digests
//! that confirm a decrypted volume key is correct.

use ring::pbkdf2;
use ring::rand::SecureRandom;

use crate::constants::DIGEST_ITERATIONS;
use crate::error::{Error, Result};
use crate::metadata::Digest;

const SHA256_LEN: usize = 32;
const SALT_LEN: usize = 32;

static PBKDF2_ALG: pbkdf2::Algorithm = pbkdf2::PBKDF2_HMAC_SHA256;

/// Creates a new PBKDF2-SHA256 digest of the volume key.
///
/// Generates a random salt, derives the digest value, and returns
/// a `Digest` metadata struct ready to be inserted into the JSON area.
pub fn create(volume_key: &[u8], keyslot_ids: &[&str], segment_ids: &[&str]) -> Result<Digest> {
    use base64ct::{Base64, Encoding};

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
///
/// Returns `Ok(true)` if the key matches, `Ok(false)` otherwise.
pub fn verify(volume_key: &[u8], digest: &Digest) -> Result<bool> {
    use base64ct::{Base64, Encoding};

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
