//! AES-XTS encryption and decryption.
//!
//! Implements IEEE P1619 XTS-AES mode using the `aes` crate for the underlying
//! block cipher. XTS splits the key into two halves: one for data encryption
//! and one for tweak encryption.

use aes::Aes256;
use aes::cipher::{BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use zeroize::Zeroize;

use crate::error::{Error, Result};

const AES_BLOCK_SIZE: usize = 16;

/// Encrypts `data` in-place using AES-256-XTS.
///
/// `key` must be 64 bytes (two 256-bit AES keys).
/// `tweak` is the 16-byte sector/unit number in little-endian.
pub fn encrypt(key: &[u8], tweak: &[u8; 16], data: &mut [u8]) -> Result<()> {
    xts_process(key, tweak, data, Mode::Encrypt)
}

/// Decrypts `data` in-place using AES-256-XTS.
///
/// `key` must be 64 bytes (two 256-bit AES keys).
/// `tweak` is the 16-byte sector/unit number in little-endian.
pub fn decrypt(key: &[u8], tweak: &[u8; 16], data: &mut [u8]) -> Result<()> {
    xts_process(key, tweak, data, Mode::Decrypt)
}

enum Mode {
    Encrypt,
    Decrypt,
}

fn xts_process(key: &[u8], tweak: &[u8; 16], data: &mut [u8], mode: Mode) -> Result<()> {
    if key.len() != 64 {
        return Err(Error::InvalidField("AES-XTS key must be 64 bytes".into()));
    }

    if data.len() < AES_BLOCK_SIZE {
        return Err(Error::InvalidField("data must be at least 16 bytes".into()));
    }

    let key1: &[u8; 32] = key[..32]
        .try_into()
        .map_err(|_| Error::InvalidField("invalid AES key length".into()))?;
    let key2: &[u8; 32] = key[32..]
        .try_into()
        .map_err(|_| Error::InvalidField("invalid AES key length".into()))?;

    let cipher1 = Aes256::new(key1.into());
    let cipher2 = Aes256::new(key2.into());

    let mut t = *tweak;
    cipher2.encrypt_block((&mut t).into());

    let full_blocks = data.len() / AES_BLOCK_SIZE;

    for i in 0..full_blocks {
        let offset = i * AES_BLOCK_SIZE;
        let block = &mut data[offset..offset + AES_BLOCK_SIZE];

        xor_block(block, &t);

        let aes_block: &mut [u8; 16] = block
            .try_into()
            .map_err(|_| Error::InvalidField("invalid block length".into()))?;
        match mode {
            Mode::Encrypt => cipher1.encrypt_block(aes_block.into()),
            Mode::Decrypt => cipher1.decrypt_block(aes_block.into()),
        }

        xor_block(block, &t);

        gf128_mul_x(&mut t);
    }

    t.zeroize();

    Ok(())
}

/// XOR a 16-byte block with a tweak value.
fn xor_block(block: &mut [u8], tweak: &[u8; 16]) {
    for (b, t) in block.iter_mut().zip(tweak.iter()) {
        *b ^= t;
    }
}

/// Multiply a value in GF(2^128) by x (left shift with reduction).
fn gf128_mul_x(tweak: &mut [u8; 16]) {
    let mut carry = 0u8;
    for byte in tweak.iter_mut() {
        let new_carry = *byte >> 7;
        *byte = (*byte << 1) | carry;
        carry = new_carry;
    }
    // If the MSB was set, reduce with the polynomial x^128 + x^7 + x^2 + x + 1 (0x87)
    tweak[0] ^= carry * 0x87;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = [0x42u8; 64]; // 64-byte key for AES-256-XTS
        let tweak = [0x01u8; 16];
        let original_data = b"Hello, World!!!!"; // 16 bytes
        let mut data = original_data.clone();

        encrypt(&key, &tweak, &mut data).unwrap();
        let encrypted = data.clone();

        assert_ne!(encrypted.as_slice(), original_data.as_slice());

        decrypt(&key, &tweak, &mut data).unwrap();

        assert_eq!(data.as_slice(), original_data.as_slice());
    }

    #[test]
    fn test_encrypt_decrypt_multiple_blocks() {
        let key = [0xABu8; 64];
        let tweak = [0x00u8; 16];
        // 64 bytes = 4 AES blocks
        let original_data = b"This is a 64 byte string for testing multiple blocks!!!";
        let mut data = original_data.clone();

        encrypt(&key, &tweak, &mut data).unwrap();
        decrypt(&key, &tweak, &mut data).unwrap();

        assert_eq!(data.as_slice(), original_data.as_slice());
    }

    #[test]
    fn test_different_tweaks_produce_different_ciphertexts() {
        let key = [0x42u8; 64];
        let tweak1 = [0x01u8; 16];
        let tweak2 = [0x02u8; 16];
        let original_data = b"Same data here!!";

        let mut data1 = original_data.clone();
        let mut data2 = original_data.clone();

        encrypt(&key, &tweak1, &mut data1).unwrap();
        encrypt(&key, &tweak2, &mut data2).unwrap();

        assert_ne!(data1.as_slice(), data2.as_slice());
    }

    #[test]
    fn test_same_tweak_produces_same_ciphertext() {
        let key = [0x42u8; 64];
        let tweak = [0x01u8; 16];
        let original_data = b"Same data here!!";

        let mut data1 = original_data.clone();
        let mut data2 = original_data.clone();

        encrypt(&key, &tweak, &mut data1).unwrap();
        encrypt(&key, &tweak, &mut data2).unwrap();

        assert_eq!(data1.as_slice(), data2.as_slice());
    }

    #[test]
    fn test_wrong_key_fails_decryption() {
        let key1 = [0x42u8; 64];
        let key2 = [0x43u8; 64];
        let tweak = [0x01u8; 16];
        let original_data = b"Secret message!!";

        let mut data = original_data.clone();
        encrypt(&key1, &tweak, &mut data).unwrap();

        let mut decrypted = data.clone();
        decrypt(&key2, &tweak, &mut decrypted).unwrap();

        assert_ne!(decrypted.as_slice(), original_data.as_slice());
    }

    #[test]
    fn test_invalid_key_size() {
        let key = [0x42u8; 32];
        let tweak = [0x01u8; 16];
        let mut data = b"Test data here!!".to_vec();

        let result = encrypt(&key, &tweak, &mut data);
        assert!(result.is_err());
    }

    #[test]
    fn test_data_too_short() {
        let key = [0x42u8; 64];
        let tweak = [0x01u8; 16];
        let mut data = b"Too short".to_vec();

        let result = encrypt(&key, &tweak, &mut data);
        assert!(result.is_err());
    }

    #[test]
    fn test_xor_block() {
        let mut block = [0xFFu8; 16];
        let tweak = [0x0Fu8; 16];

        xor_block(&mut block, &tweak);

        let expected = [0xF0u8; 16];
        assert_eq!(block, expected);
    }

    #[test]
    fn test_xor_block_zero_tweak_unchanged() {
        let mut block = [0xABu8; 16];
        let tweak = [0x00u8; 16];

        xor_block(&mut block, &tweak);

        assert_eq!(block, [0xABu8; 16]);
    }

    #[test]
    fn test_gf128_mul_x_simple() {
        let mut tweak = [0x00u8; 16];
        tweak[15] = 0x02;

        gf128_mul_x(&mut tweak);

        // 0x02 << 1 = 0x04
        assert_eq!(tweak[15], 0x04);
    }

    #[test]
    fn test_gf128_mul_x_with_carry() {
        // Test carry propagation
        let mut tweak = [0x00u8; 16];
        tweak[15] = 0x80; // MSB set
        tweak[14] = 0x01;

        gf128_mul_x(&mut tweak);

        // 0x80 << 1 = 0x00 with carry 1
        // 0x01 << 1 + carry = 0x03
        assert_eq!(tweak[15], 0x00);
        assert_eq!(tweak[14], 0x02);
    }

    #[test]
    fn test_gf128_mul_x_reduction() {
        // Test reduction when MSB of byte 0 is set
        let mut tweak = [0x80u8; 16];

        gf128_mul_x(&mut tweak);

        // MSB was set, so we should have reduction
        // After shift, byte 0 should be 0x00 ^ 0x87 = 0x87
        assert_eq!(tweak[0], 0x87);
    }

    #[test]
    fn test_gf128_mul_x_full_reduction() {
        // Test with all 1s - should produce specific reduction pattern
        let mut tweak = [0xFFu8; 16];

        gf128_mul_x(&mut tweak);

        // With all bits set, after shift we get:
        // Each byte becomes (byte << 1) | carry from previous
        // Byte 0 gets reduction: 0xFE ^ 0x87 = 0x79
        assert_eq!(tweak[0], 0x79);
    }

    #[test]
    fn test_tweak_affects_encryption() {
        let key = [0x42u8; 64];
        let tweak1 = [0x00u8; 16];
        let mut tweak2 = [0x00u8; 16];
        tweak2[15] = 0x01; // Different tweak value

        let original_data = b"This is a 64 byte test string for verifying tweak behavior!!";
        let mut data1 = original_data.clone();
        let mut data2 = original_data.clone();

        encrypt(&key, &tweak1, &mut data1).unwrap();
        encrypt(&key, &tweak2, &mut data2).unwrap();

        assert_ne!(data1.as_slice(), data2.as_slice());
    }
}
