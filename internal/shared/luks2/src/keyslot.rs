//! LUKS2 keyslot operations.
//!
//! Handles volume key protection: deriving intermediate keys from passphrases
//! via Argon2id, anti-forensic splitting/merging for secure key storage, and
//! AES-XTS encryption/decryption of keyslot binary areas.

use base64ct::{Base64, Encoding};
use ring::digest::{Context, SHA256};
use ring::rand::SecureRandom;
use zeroize::Zeroize;

use crate::crypto;
use crate::error::{Error, Result};
use crate::metadata::Keyslot;

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
///
/// Each stripe is diffused so that overwriting any single stripe on disk
/// renders the entire key unrecoverable.
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
///
/// Processes the buffer in SHA256_LEN-sized chunks, hashing each chunk
/// with a counter prefix to diffuse the data.
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
///
/// 1. Derives intermediate key from passphrase via Argon2id
/// 2. AF-splits the volume key into stripes
/// 3. Encrypts the striped data with AES-XTS using the derived key
pub fn encrypt_keyslot(passphrase: &[u8], volume_key: &[u8], keyslot: &Keyslot) -> Result<Vec<u8>> {
    let mut derived_key = derive_key(passphrase, keyslot)?;
    let mut striped = af_split(volume_key, keyslot.af.stripes)?;

    let tweak = [0u8; 16];
    crypto::encrypt(&derived_key, &tweak, &mut striped)?;

    derived_key.zeroize();
    Ok(striped)
}

/// Decrypts keyslot binary data to recover a volume key candidate.
///
/// 1. Derives intermediate key from passphrase via Argon2id
/// 2. Decrypts the keyslot area with AES-XTS
/// 3. AF-merges the stripes to recover the volume key candidate
pub fn decrypt_keyslot(
    passphrase: &[u8],
    keyslot: &Keyslot,
    encrypted_data: &[u8],
) -> Result<Vec<u8>> {
    let mut derived_key = derive_key(passphrase, keyslot)?;

    let mut data = encrypted_data.to_vec();
    let tweak = [0u8; 16];
    crypto::decrypt(&derived_key, &tweak, &mut data)?;

    derived_key.zeroize();

    let key_size = keyslot.key_size as usize;
    let volume_key = af_merge(&data, key_size, keyslot.af.stripes)?;
    data.zeroize();

    Ok(volume_key)
}
