//! LUKS2 keyslot operations.
//!
//! Handles volume key protection: deriving intermediate keys from passphrases
//! via Argon2id, anti-forensic splitting/merging for secure key storage, and
//! AES-XTS encryption/decryption of keyslot binary areas.

use base64ct::{Base64, Encoding};
use ring::digest::{Context, SHA256};
use ring::rand::SecureRandom;
use zeroize::Zeroize;

use crate::error::{Error, Result};
use crate::metadata::Keyslot;
use crate::xts;

const SHA256_LEN: usize = 32;

/// Derives an intermediate key from a passphrase using Argon2id.
pub fn derive_key(passphrase: &[u8], keyslot: &Keyslot) -> Result<Vec<u8>> {
    if keyslot.kdf.r#type != "argon2id" {
        return Err(Error::UnsupportedKdf(keyslot.kdf.r#type.clone()));
    }

    let salt = Base64::decode_vec(&keyslot.kdf.salt)?;

    let t_cost = keyslot.kdf.time.unwrap_or(4);
    let m_cost = keyslot.kdf.memory.unwrap_or(1_048_576);
    let p_cost = keyslot.kdf.cpus.unwrap_or(4);

    let params = argon2::Params::new(m_cost, t_cost, p_cost, Some(keyslot.key_size as usize))
        .map_err(Error::Argon2)?;

    let argon2 = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);

    let mut derived = vec![0u8; keyslot.key_size as usize];
    argon2.hash_password_into(passphrase, &salt, &mut derived)?;

    Ok(derived)
}

/// Anti-forensic split: expand a volume key into `stripes` * key_size bytes.
pub fn af_split(key: &[u8], stripes: u32) -> Result<Vec<u8>> {
    let key_size = key.len();
    let total = key_size * stripes as usize;
    let mut buf = vec![0u8; total];

    let rng = ring::rand::SystemRandom::new();

    // Generate random data for all stripes except the last
    for i in 0..(stripes as usize - 1) {
        let offset = i * key_size;
        rng.fill(&mut buf[offset..offset + key_size])
            .map_err(|_| Error::InvalidField("random generation failed".into()))?;
    }

    // Compute the diffusion digest of all stripes except the last
    let mut d = vec![0u8; key_size];
    for i in 0..(stripes as usize - 1) {
        let offset = i * key_size;
        for (j, byte) in d.iter_mut().enumerate() {
            *byte ^= buf[offset + j];
        }
        af_diffuse(&mut d);
    }

    // The last stripe is key XOR diffused-sum so that merge recovers the key
    let last_offset = (stripes as usize - 1) * key_size;
    for (j, byte) in d.iter().enumerate() {
        buf[last_offset + j] = key[j] ^ byte;
    }

    Ok(buf)
}

/// Anti-forensic merge: recover the volume key from split stripes.
pub fn af_merge(data: &[u8], key_size: usize, stripes: u32) -> Result<Vec<u8>> {
    if data.len() != key_size * stripes as usize {
        return Err(Error::InvalidField("AF data size mismatch".into()));
    }

    let mut d = vec![0u8; key_size];

    for i in 0..stripes as usize {
        let offset = i * key_size;
        for (j, byte) in d.iter_mut().enumerate() {
            *byte ^= data[offset + j];
        }
        if i < (stripes as usize - 1) {
            af_diffuse(&mut d);
        }
    }

    Ok(d)
}

/// SHA-256 based diffusion function for anti-forensic splitting.
fn af_diffuse(data: &mut [u8]) {
    let chunks = data.len() / SHA256_LEN;
    let remainder = data.len() % SHA256_LEN;

    for i in 0..chunks {
        let offset = i * SHA256_LEN;
        let mut ctx = Context::new(&SHA256);
        ctx.update(&(i as u32).to_be_bytes());
        ctx.update(&data[offset..offset + SHA256_LEN]);
        let hash = ctx.finish();
        data[offset..offset + SHA256_LEN].copy_from_slice(&hash.as_ref()[..SHA256_LEN]);
    }

    if remainder > 0 {
        let offset = chunks * SHA256_LEN;
        let mut ctx = Context::new(&SHA256);
        ctx.update(&(chunks as u32).to_be_bytes());
        ctx.update(&data[offset..offset + remainder]);
        let hash = ctx.finish();
        data[offset..offset + remainder].copy_from_slice(&hash.as_ref()[..remainder]);
    }
}

/// Encrypts a volume key into keyslot binary data ready to be written to disk.
pub fn encrypt_keyslot(passphrase: &[u8], volume_key: &[u8], keyslot: &Keyslot) -> Result<Vec<u8>> {
    let mut derived_key = derive_key(passphrase, keyslot)?;
    let mut striped = af_split(volume_key, keyslot.af.stripes)?;

    let tweak = [0u8; 16];
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
    let tweak = [0u8; 16];
    xts::decrypt(&derived_key, &tweak, &mut data)?;

    derived_key.zeroize();

    let key_size = keyslot.key_size as usize;
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
    fn test_af_split_merge_roundtrip() {
        let key = vec![0xABu8; 64];
        let stripes = 100;

        let split = af_split(&key, stripes).unwrap();
        let merged = af_merge(&split, key.len(), stripes).unwrap();

        assert_eq!(key, merged);
    }

    #[test]
    fn test_af_split_different_stripes() {
        let key = vec![0x42u8; 32];

        for stripes in [1, 10, 100, 4000] {
            let split = af_split(&key, stripes).unwrap();
            assert_eq!(split.len(), key.len() * stripes as usize);

            let merged = af_merge(&split, key.len(), stripes).unwrap();
            assert_eq!(key, merged);
        }
    }

    #[test]
    fn test_af_merge_wrong_size() {
        let data = vec![0x42u8; 100];
        let result = af_merge(&data, 64, 4000);
        assert!(result.is_err());
    }

    #[test]
    fn test_af_split_changes_with_same_key() {
        let key = vec![0x42u8; 64];
        let stripes = 100;

        let split1 = af_split(&key, stripes).unwrap();
        let split2 = af_split(&key, stripes).unwrap();

        assert_ne!(split1, split2);

        let merged1 = af_merge(&split1, key.len(), stripes).unwrap();
        let merged2 = af_merge(&split2, key.len(), stripes).unwrap();

        assert_eq!(merged1, key);
        assert_eq!(merged2, key);
    }

    #[test]
    fn test_derive_key_unsupported_kdf() {
        let mut keyslot = create_test_keyslot();
        keyslot.kdf.r#type = "pbkdf2".to_string();

        let result = derive_key(b"password", &keyslot);
        assert!(result.is_err());
    }

    #[test]
    fn test_derive_key_same_passphrase_same_result() {
        let keyslot = create_test_keyslot();
        let passphrase = b"test_password";

        let derived1 = derive_key(passphrase, &keyslot).unwrap();
        let derived2 = derive_key(passphrase, &keyslot).unwrap();

        assert_eq!(derived1, derived2);
    }

    #[test]
    fn test_derive_key_different_passphrase_different_result() {
        let keyslot = create_test_keyslot();

        let derived1 = derive_key(b"password1", &keyslot).unwrap();
        let derived2 = derive_key(b"password2", &keyslot).unwrap();

        assert_ne!(derived1, derived2);
    }

    #[test]
    fn test_derive_key_produces_expected_size() {
        let keyslot = create_test_keyslot();
        let passphrase = b"test_password";

        let derived = derive_key(passphrase, &keyslot).unwrap();

        assert_eq!(derived.len(), keyslot.key_size as usize);
    }

    #[test]
    fn test_af_diffuse_deterministic() {
        let mut data1 = vec![0x42u8; 64];
        let mut data2 = data1.clone();

        af_diffuse(&mut data1);
        af_diffuse(&mut data2);

        assert_eq!(data1, data2);
    }

    #[test]
    fn test_af_diffuse_changes_data() {
        let original = vec![0x42u8; 64];
        let mut data = original.clone();

        af_diffuse(&mut data);

        assert_ne!(data, original);
    }

    #[test]
    fn test_encrypt_decrypt_keyslot_roundtrip() {
        let keyslot = create_test_keyslot();
        let passphrase = b"test_password";
        let volume_key = vec![0xABu8; 64];

        let encrypted = encrypt_keyslot(passphrase, &volume_key, &keyslot).unwrap();
        let decrypted = decrypt_keyslot(passphrase, &keyslot, &encrypted).unwrap();

        assert_eq!(decrypted, volume_key);
    }

    #[test]
    fn test_decrypt_keyslot_wrong_passphrase() {
        let keyslot = create_test_keyslot();
        let correct_passphrase = b"correct_password";
        let wrong_passphrase = b"wrong_password";
        let volume_key = vec![0xABu8; 64];

        let encrypted = encrypt_keyslot(correct_passphrase, &volume_key, &keyslot).unwrap();

        let result = decrypt_keyslot(wrong_passphrase, &keyslot, &encrypted);

        // Should either fail or produce garbage (but not panic)
        if let Ok(decrypted) = result {
            assert_ne!(decrypted, volume_key);
        }
    }

    #[test]
    fn test_encrypt_keyslot_produces_different_output() {
        let keyslot = create_test_keyslot();
        let passphrase = b"test_password";
        let volume_key = vec![0xABu8; 64];

        let encrypted1 = encrypt_keyslot(passphrase, &volume_key, &keyslot).unwrap();
        let encrypted2 = encrypt_keyslot(passphrase, &volume_key, &keyslot).unwrap();

        assert_ne!(encrypted1, encrypted2);
    }

    #[test]
    fn test_af_split_minimum_stripes() {
        let key = vec![0x42u8; 64];
        let stripes = 1;

        let split = af_split(&key, stripes).unwrap();
        assert_eq!(split.len(), key.len());

        let merged = af_merge(&split, key.len(), stripes).unwrap();
        assert_eq!(key, merged);
    }

    #[test]
    fn test_af_merge_with_corrupted_data() {
        let key = vec![0x42u8; 64];
        let stripes = 100;

        let mut split = af_split(&key, stripes).unwrap();
        split[0] ^= 0xFF;
        split[10] ^= 0xFF;

        let merged = af_merge(&split, key.len(), stripes).unwrap();

        assert_ne!(merged, key);
    }

    #[test]
    fn test_derive_key_empty_passphrase() {
        let keyslot = create_test_keyslot();
        let passphrase = b"";

        let result = derive_key(passphrase, &keyslot);
        assert!(result.is_ok());

        let derived = result.unwrap();
        assert_eq!(derived.len(), keyslot.key_size as usize);
    }

    #[test]
    fn test_derive_key_long_passphrase() {
        let keyslot = create_test_keyslot();
        let passphrase = vec![0x42u8; 1000];

        let result = derive_key(&passphrase, &keyslot);
        assert!(result.is_ok());

        let derived = result.unwrap();
        assert_eq!(derived.len(), keyslot.key_size as usize);
    }
}
