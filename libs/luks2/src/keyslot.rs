//! LUKS2 keyslot operations.

use core::num::NonZeroUsize;

use base64ct::{Base64, Encoding as _};
use ring::digest::{Context, SHA256};
use ring::rand::{SecureRandom as _, SystemRandom};
use zeroize::Zeroize as _;

use crate::error::{Luks2Error, Result};
use crate::metadata::Keyslot;
use crate::xts;

const SHA256_LEN: usize = 32;

/// Derives an intermediate key from a passphrase using Argon2id.
pub fn derive_key(passphrase: &[u8], keyslot: &Keyslot) -> Result<Vec<u8>> {
    if keyslot.kdf.r#type != "argon2id" {
        return Err(Luks2Error::UnsupportedKdf(keyslot.kdf.r#type.clone()));
    }

    let salt = Base64::decode_vec(&keyslot.kdf.salt)?;
    let key_size = usize::try_from(keyslot.key_size)
        .map_err(|_error| Luks2Error::InvalidField("invalid key size".into()))?;

    let t_cost = keyslot.kdf.time.unwrap_or(4);
    let m_cost = keyslot.kdf.memory.unwrap_or(1_048_576);
    let p_cost = keyslot.kdf.cpus.unwrap_or(4);

    let params =
        argon2::Params::new(m_cost, t_cost, p_cost, Some(key_size)).map_err(Luks2Error::Argon2)?;

    let argon2 = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);

    let mut derived = vec![0_u8; key_size];
    argon2.hash_password_into(passphrase, &salt, &mut derived)?;

    Ok(derived)
}

/// Anti-forensic split: expand a volume key into `stripes` * `key_size` bytes.
pub fn af_split(key: &[u8], stripes: u32) -> Result<Vec<u8>> {
    let key_size = key.len();
    let stripes = usize::try_from(stripes)
        .map_err(|_error| Luks2Error::InvalidField("invalid stripe count".into()))?;
    let total = key_size
        .checked_mul(stripes)
        .ok_or_else(|| Luks2Error::InvalidField("AF split size overflow".into()))?;
    let mut buf = vec![0_u8; total];

    let rng = SystemRandom::new();

    // Generate random data for all stripes except the last
    for stripe in buf
        .chunks_exact_mut(key_size)
        .take(stripes.saturating_sub(1))
    {
        rng.fill(stripe)
            .map_err(|_error| Luks2Error::InvalidField("random generation failed".into()))?;
    }

    // Compute the diffusion digest of all stripes except the last
    let mut diffusion = vec![0_u8; key_size];
    for stripe in buf.chunks_exact(key_size).take(stripes.saturating_sub(1)) {
        for (byte, stripe_byte) in diffusion.iter_mut().zip(stripe.iter().copied()) {
            *byte ^= stripe_byte;
        }
        af_diffuse(&mut diffusion)?;
    }

    // The last stripe is key XOR diffused-sum so that merge recovers the key
    let last_stripe = buf
        .chunks_exact_mut(key_size)
        .nth(stripes.saturating_sub(1))
        .ok_or_else(|| Luks2Error::InvalidField("missing last stripe".into()))?;
    for ((dst, key_byte), byte) in last_stripe
        .iter_mut()
        .zip(key.iter().copied())
        .zip(diffusion.iter().copied())
    {
        *dst = key_byte ^ byte;
    }

    Ok(buf)
}

/// Anti-forensic merge: recover the volume key from split stripes.
pub fn af_merge(data: &[u8], key_size: usize, stripes: u32) -> Result<Vec<u8>> {
    let stripes = usize::try_from(stripes)
        .map_err(|_error| Luks2Error::InvalidField("invalid stripe count".into()))?;
    let expected_len = key_size
        .checked_mul(stripes)
        .ok_or_else(|| Luks2Error::InvalidField("AF merge size overflow".into()))?;
    if data.len() != expected_len {
        return Err(Luks2Error::InvalidField("AF data size mismatch".into()));
    }

    let mut diffusion = vec![0_u8; key_size];

    for (index, stripe) in data.chunks_exact(key_size).enumerate() {
        for (byte, stripe_byte) in diffusion.iter_mut().zip(stripe.iter().copied()) {
            *byte ^= stripe_byte;
        }
        if index < stripes.saturating_sub(1) {
            af_diffuse(&mut diffusion)?;
        }
    }

    Ok(diffusion)
}

/// SHA-256 based diffusion function for anti-forensic splitting.
fn af_diffuse(data: &mut [u8]) -> Result<()> {
    let mut chunks = data.chunks_exact_mut(SHA256_LEN);
    let mut chunk_count = 0_usize;

    for (index, chunk) in chunks.by_ref().enumerate() {
        let mut ctx = Context::new(&SHA256);
        let index = u32::try_from(index)
            .map_err(|_error| Luks2Error::InvalidField("too many AF chunks".into()))?;
        ctx.update(&index.to_be_bytes());
        ctx.update(chunk);
        let hash = ctx.finish();
        let hash_prefix = hash
            .as_ref()
            .get(..SHA256_LEN)
            .ok_or_else(|| Luks2Error::InvalidField("hash shorter than chunk".into()))?;
        chunk.copy_from_slice(hash_prefix);
        chunk_count = chunk_count.saturating_add(1);
    }

    let remainder = chunks.into_remainder();
    if let Some(remainder_len) = NonZeroUsize::new(remainder.len()) {
        let mut ctx = Context::new(&SHA256);
        let index = u32::try_from(chunk_count)
            .map_err(|_error| Luks2Error::InvalidField("too many AF chunks".into()))?;
        ctx.update(&index.to_be_bytes());
        ctx.update(remainder);
        let hash = ctx.finish();
        let hash_prefix = hash
            .as_ref()
            .get(..remainder_len.get())
            .ok_or_else(|| Luks2Error::InvalidField("hash shorter than remainder".into()))?;
        remainder.copy_from_slice(hash_prefix);
    }

    Ok(())
}

/// Encrypts a volume key into keyslot binary data ready to be written to disk.
pub fn encrypt_keyslot(passphrase: &[u8], volume_key: &[u8], keyslot: &Keyslot) -> Result<Vec<u8>> {
    let mut derived_key = derive_key(passphrase, keyslot)?;
    let mut striped = af_split(volume_key, keyslot.af.stripes)?;

    let tweak = [0_u8; 16];
    xts::encrypt(&derived_key, &tweak, &mut striped)?;

    derived_key.zeroize();
    Ok(striped)
}

/// Decrypts keyslot binary data to recover a volume key candidate.
pub fn decrypt_keyslot(
    passphrase: &[u8],
    keyslot: &Keyslot,
    encrypted_data: &[u8],
) -> Result<Vec<u8>> {
    let mut derived_key = derive_key(passphrase, keyslot)?;

    let mut data = encrypted_data.to_vec();
    let tweak = [0_u8; 16];
    xts::decrypt(&derived_key, &tweak, &mut data)?;

    derived_key.zeroize();

    let key_size = usize::try_from(keyslot.key_size)
        .map_err(|_error| Luks2Error::InvalidField("invalid key size".into()))?;
    let volume_key = af_merge(&data, key_size, keyslot.af.stripes)?;
    data.zeroize();

    Ok(volume_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{AntiForensic, Kdf, KeyslotArea};

    fn create_test_keyslot() -> Keyslot {
        Keyslot {
            r#type: "luks2".to_string(),
            key_size: 64,
            kdf: Kdf {
                r#type: "argon2id".to_string(),
                salt: base64ct::Base64::encode_string(&[0x42u8; 64]),
                time: Some(1),
                memory: Some(65536),
                cpus: Some(4),
            },
            af: AntiForensic {
                r#type: "luks1".to_string(),
                stripes: 4000,
                hash: "sha256".to_string(),
            },
            area: KeyslotArea {
                r#type: "raw".to_string(),
                offset: "32768".to_string(),
                size: "262144000".to_string(),
                encryption: "aes-xts-plain64".to_string(),
                key_size: 64,
            },
        }
    }

    #[test]
    fn af_split_merge_roundtrip() {
        // ARRANGE
        let key = vec![0xABu8; 64];
        let stripes = 100;

        // ACT
        let split = af_split(&key, stripes).unwrap();
        let merged = af_merge(&split, key.len(), stripes).unwrap();

        // ASSERT
        assert_eq!(key, merged);
    }

    #[test]
    fn af_split_different_stripes() {
        // ARRANGE
        let key = vec![0x42u8; 32];

        // ACT & ASSERT
        for stripes in [1, 10, 100, 4000] {
            let split = af_split(&key, stripes).unwrap();
            assert_eq!(split.len(), key.len() * stripes as usize);

            let merged = af_merge(&split, key.len(), stripes).unwrap();
            assert_eq!(key, merged);
        }
    }

    #[test]
    fn af_merge_wrong_size() {
        // ARRANGE
        let data = vec![0x42u8; 100];

        // ACT
        let result = af_merge(&data, 64, 4000);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn af_split_changes_with_same_key() {
        // ARRANGE
        let key = vec![0x42u8; 64];
        let stripes = 100;

        // ACT
        let split1 = af_split(&key, stripes).unwrap();
        let split2 = af_split(&key, stripes).unwrap();

        // ASSERT
        assert_ne!(split1, split2);

        let merged1 = af_merge(&split1, key.len(), stripes).unwrap();
        let merged2 = af_merge(&split2, key.len(), stripes).unwrap();

        assert_eq!(merged1, key);
        assert_eq!(merged2, key);
    }

    #[test]
    fn derive_key_unsupported_kdf() {
        // ARRANGE
        let mut keyslot = create_test_keyslot();
        keyslot.kdf.r#type = "pbkdf2".to_string();

        // ACT
        let result = derive_key(b"password", &keyslot);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn derive_key_same_passphrase_same_result() {
        // ARRANGE
        let keyslot = create_test_keyslot();
        let passphrase = b"test_password";

        // ACT
        let derived1 = derive_key(passphrase, &keyslot).unwrap();
        let derived2 = derive_key(passphrase, &keyslot).unwrap();

        // ASSERT
        assert_eq!(derived1, derived2);
    }

    #[test]
    fn derive_key_different_passphrase_different_result() {
        // ARRANGE
        let keyslot = create_test_keyslot();

        // ACT
        let derived1 = derive_key(b"password1", &keyslot).unwrap();
        let derived2 = derive_key(b"password2", &keyslot).unwrap();

        // ASSERT
        assert_ne!(derived1, derived2);
    }

    #[test]
    fn derive_key_produces_expected_size() {
        // ARRANGE
        let keyslot = create_test_keyslot();
        let passphrase = b"test_password";

        // ACT
        let derived = derive_key(passphrase, &keyslot).unwrap();

        // ASSERT
        assert_eq!(derived.len(), keyslot.key_size as usize);
    }

    #[test]
    fn af_diffuse_deterministic() {
        // ARRANGE
        let mut data1 = vec![0x42u8; 64];
        let mut data2 = data1.clone();

        // ACT
        af_diffuse(&mut data1).unwrap();
        af_diffuse(&mut data2).unwrap();

        // ASSERT
        assert_eq!(data1, data2);
    }

    #[test]
    fn af_diffuse_changes_data() {
        // ARRANGE
        let original = vec![0x42u8; 64];
        let mut data = original.clone();

        // ACT
        af_diffuse(&mut data).unwrap();

        // ASSERT
        assert_ne!(data, original);
    }

    #[test]
    fn encrypt_decrypt_keyslot_roundtrip() {
        // ARRANGE
        let keyslot = create_test_keyslot();
        let passphrase = b"test_password";
        let volume_key = vec![0xABu8; 64];

        // ACT
        let encrypted = encrypt_keyslot(passphrase, &volume_key, &keyslot).unwrap();
        let decrypted = decrypt_keyslot(passphrase, &keyslot, &encrypted).unwrap();

        // ASSERT
        assert_eq!(decrypted, volume_key);
    }

    #[test]
    fn decrypt_keyslot_wrong_passphrase() {
        // ARRANGE
        let keyslot = create_test_keyslot();
        let correct_passphrase = b"correct_password";
        let wrong_passphrase = b"wrong_password";
        let volume_key = vec![0xABu8; 64];

        let encrypted = encrypt_keyslot(correct_passphrase, &volume_key, &keyslot).unwrap();

        // ACT
        let result = decrypt_keyslot(wrong_passphrase, &keyslot, &encrypted);

        // ASSERT
        assert!(!matches!(result, Ok(decrypted) if decrypted == volume_key));
    }

    #[test]
    fn af_diffuse_handles_remainder_chunk() {
        // ARRANGE
        let mut data = vec![0x42_u8; 33];
        let original = data.clone();

        // ACT
        af_diffuse(&mut data).unwrap();

        // ASSERT
        assert_ne!(data, original);
        assert_eq!(data.len(), 33);
    }

    #[test]
    fn encrypt_keyslot_produces_different_output() {
        // ARRANGE
        let keyslot = create_test_keyslot();
        let passphrase = b"test_password";
        let volume_key = vec![0xABu8; 64];

        // ACT
        let encrypted1 = encrypt_keyslot(passphrase, &volume_key, &keyslot).unwrap();
        let encrypted2 = encrypt_keyslot(passphrase, &volume_key, &keyslot).unwrap();

        // ASSERT
        assert_ne!(encrypted1, encrypted2);
    }

    #[test]
    fn af_split_minimum_stripes() {
        // ARRANGE
        let key = vec![0x42u8; 64];
        let stripes = 1;

        // ACT
        let split = af_split(&key, stripes).unwrap();

        // ASSERT
        assert_eq!(split.len(), key.len());

        let merged = af_merge(&split, key.len(), stripes).unwrap();
        assert_eq!(key, merged);
    }

    #[test]
    fn af_merge_with_corrupted_data() {
        // ARRANGE
        let key = vec![0x42u8; 64];
        let stripes = 100;

        let mut split = af_split(&key, stripes).unwrap();
        split[0] ^= 0xFF;
        split[10] ^= 0xFF;

        // ACT
        let merged = af_merge(&split, key.len(), stripes).unwrap();

        // ASSERT
        assert_ne!(merged, key);
    }

    #[test]
    fn derive_key_empty_passphrase() {
        // ARRANGE
        let keyslot = create_test_keyslot();
        let passphrase = b"";

        // ACT
        let result = derive_key(passphrase, &keyslot);

        // ASSERT
        assert!(result.is_ok());

        let derived = result.unwrap();
        assert_eq!(derived.len(), keyslot.key_size as usize);
    }

    #[test]
    fn derive_key_long_passphrase() {
        // ARRANGE
        let keyslot = create_test_keyslot();
        let passphrase = vec![0x42u8; 1000];

        // ACT
        let result = derive_key(&passphrase, &keyslot);

        // ASSERT
        assert!(result.is_ok());

        let derived = result.unwrap();
        assert_eq!(derived.len(), keyslot.key_size as usize);
    }
}
