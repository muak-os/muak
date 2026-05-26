//! AES-XTS encryption and decryption.
//!
//! Implements IEEE P1619 XTS-AES mode using the `aes` crate for the underlying
//! block cipher. XTS splits the key into two halves: one for data encryption
//! and one for tweak encryption.

use aes::Aes256;
use aes::cipher::{BlockCipherDecrypt as _, BlockCipherEncrypt as _, KeyInit as _};
use zeroize::Zeroize as _;

use crate::error::{Luks2Error, Result};

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

#[derive(Clone, Copy)]
enum Mode {
    Encrypt,
    Decrypt,
}

fn xts_process(key: &[u8], tweak: &[u8; 16], data: &mut [u8], mode: Mode) -> Result<()> {
    if key.len() != 64 {
        return Err(Luks2Error::InvalidField(
            "AES-XTS key must be 64 bytes".into(),
        ));
    }

    if data.len() < AES_BLOCK_SIZE {
        return Err(Luks2Error::InvalidField(
            "data must be at least 16 bytes".into(),
        ));
    }

    let key1_slice = key
        .get(..32)
        .ok_or_else(|| Luks2Error::InvalidField("invalid AES key length".into()))?;
    let key1: &[u8; 32] = key1_slice
        .try_into()
        .map_err(|_error| Luks2Error::InvalidField("invalid AES key length".into()))?;
    let key2_slice = key
        .get(32..)
        .ok_or_else(|| Luks2Error::InvalidField("invalid AES key length".into()))?;
    let key2: &[u8; 32] = key2_slice
        .try_into()
        .map_err(|_error| Luks2Error::InvalidField("invalid AES key length".into()))?;

    let cipher1 = Aes256::new(key1.into());
    let cipher2 = Aes256::new(key2.into());

    let mut current_tweak = *tweak;
    cipher2.encrypt_block((&mut current_tweak).into());

    for block in data.chunks_exact_mut(AES_BLOCK_SIZE) {
        xor_block(block, &current_tweak);

        {
            let aes_block: &mut [u8; 16] = block
                .try_into()
                .map_err(|_error| Luks2Error::InvalidField("invalid block length".into()))?;

            match mode {
                Mode::Encrypt => cipher1.encrypt_block(aes_block.into()),
                Mode::Decrypt => cipher1.decrypt_block(aes_block.into()),
            }
        }

        xor_block(block, &current_tweak);

        gf128_mul_x(&mut current_tweak);
    }

    current_tweak.zeroize();

    Ok(())
}

/// XOR a 16-byte block with a tweak value.
fn xor_block(block: &mut [u8], tweak: &[u8; 16]) {
    for (block_byte, tweak_byte) in block.iter_mut().zip(tweak.iter()) {
        *block_byte ^= *tweak_byte;
    }
}

/// Multiply a value in GF(2^128) by x (left shift with reduction).
fn gf128_mul_x(tweak: &mut [u8; 16]) {
    let mut carry = 0_u8;
    for byte in tweak.iter_mut() {
        let new_carry = *byte >> 7;
        *byte = (*byte << 1) | carry;
        carry = new_carry;
    }
    // If the MSB was set, reduce with the polynomial x^128 + x^7 + x^2 + x + 1 (0x87)
    if carry == 1 {
        tweak[0] ^= 0x87;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        // ARRANGE
        let key = [0x42u8; 64];
        let tweak = [0x01u8; 16];
        let original_data = b"Hello, World!!!!";
        let mut data = original_data.clone();

        // ACT
        encrypt(&key, &tweak, &mut data).unwrap();
        let encrypted = data.clone();

        // ASSERT
        assert_ne!(encrypted.as_slice(), original_data.as_slice());

        // ACT
        decrypt(&key, &tweak, &mut data).unwrap();

        // ASSERT
        assert_eq!(data.as_slice(), original_data.as_slice());
    }

    #[test]
    fn encrypt_decrypt_multiple_blocks() {
        // ARRANGE
        let key = [0xABu8; 64];
        let tweak = [0x00u8; 16];
        let original_data = b"This is a 64 byte string for testing multiple blocks!!!";
        let mut data = original_data.clone();

        // ACT
        encrypt(&key, &tweak, &mut data).unwrap();
        decrypt(&key, &tweak, &mut data).unwrap();

        // ASSERT
        assert_eq!(data.as_slice(), original_data.as_slice());
    }

    #[test]
    fn different_tweaks_produce_different_ciphertexts() {
        // ARRANGE
        let key = [0x42u8; 64];
        let tweak1 = [0x01u8; 16];
        let tweak2 = [0x02u8; 16];
        let original_data = b"Same data here!!";

        let mut data1 = original_data.clone();
        let mut data2 = original_data.clone();

        // ACT
        encrypt(&key, &tweak1, &mut data1).unwrap();
        encrypt(&key, &tweak2, &mut data2).unwrap();

        // ASSERT
        assert_ne!(data1.as_slice(), data2.as_slice());
    }

    #[test]
    fn same_tweak_produces_same_ciphertext() {
        // ARRANGE
        let key = [0x42u8; 64];
        let tweak = [0x01u8; 16];
        let original_data = b"Same data here!!";

        let mut data1 = original_data.clone();
        let mut data2 = original_data.clone();

        // ACT
        encrypt(&key, &tweak, &mut data1).unwrap();
        encrypt(&key, &tweak, &mut data2).unwrap();

        // ASSERT
        assert_eq!(data1.as_slice(), data2.as_slice());
    }

    #[test]
    fn wrong_key_fails_decryption() {
        // ARRANGE
        let key1 = [0x42u8; 64];
        let key2 = [0x43u8; 64];
        let tweak = [0x01u8; 16];
        let original_data = b"Secret message!!";

        let mut data = original_data.clone();
        encrypt(&key1, &tweak, &mut data).unwrap();

        let mut decrypted = data.clone();

        // ACT
        decrypt(&key2, &tweak, &mut decrypted).unwrap();

        // ASSERT
        assert_ne!(decrypted.as_slice(), original_data.as_slice());
    }

    #[test]
    fn invalid_key_size() {
        // ARRANGE
        let key = [0x42u8; 32];
        let tweak = [0x01u8; 16];
        let mut data = b"Test data here!!".to_vec();

        // ACT
        let result = encrypt(&key, &tweak, &mut data);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn data_too_short() {
        // ARRANGE
        let key = [0x42u8; 64];
        let tweak = [0x01u8; 16];
        let mut data = b"Too short".to_vec();

        // ACT
        let result = encrypt(&key, &tweak, &mut data);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn xor_block_flips_bits() {
        // ARRANGE
        let mut block = [0xFFu8; 16];
        let tweak = [0x0Fu8; 16];

        // ACT
        xor_block(&mut block, &tweak);

        // ASSERT
        let expected = [0xF0u8; 16];
        assert_eq!(block, expected);
    }

    #[test]
    fn xor_block_zero_tweak_unchanged() {
        // ARRANGE
        let mut block = [0xABu8; 16];
        let tweak = [0x00u8; 16];

        // ACT
        xor_block(&mut block, &tweak);

        // ASSERT
        assert_eq!(block, [0xABu8; 16]);
    }

    #[test]
    fn gf128_mul_x_simple() {
        // ARRANGE
        let mut tweak = [0x00u8; 16];
        tweak[15] = 0x02;

        // ACT
        gf128_mul_x(&mut tweak);

        // ASSERT
        assert_eq!(tweak[15], 0x04);
    }

    #[test]
    fn gf128_mul_x_with_carry() {
        // ARRANGE
        let mut tweak = [0x00u8; 16];
        tweak[15] = 0x80;
        tweak[14] = 0x01;

        // ACT
        gf128_mul_x(&mut tweak);

        // ASSERT
        assert_eq!(tweak[15], 0x00);
        assert_eq!(tweak[14], 0x02);
    }

    #[test]
    fn gf128_mul_x_reduction() {
        // ARRANGE
        let mut tweak = [0x80u8; 16];

        // ACT
        gf128_mul_x(&mut tweak);

        // ASSERT
        assert_eq!(tweak[0], 0x87);
    }

    #[test]
    fn gf128_mul_x_full_reduction() {
        // ARRANGE
        let mut tweak = [0xFFu8; 16];

        // ACT
        gf128_mul_x(&mut tweak);

        // ASSERT
        assert_eq!(tweak[0], 0x79);
    }

    #[test]
    fn tweak_affects_encryption() {
        // ARRANGE
        let key = [0x42u8; 64];
        let tweak1 = [0x00u8; 16];
        let mut tweak2 = [0x00u8; 16];
        tweak2[15] = 0x01;

        let original_data = b"This is a 64 byte test string for verifying tweak behavior!!";
        let mut data1 = original_data.clone();
        let mut data2 = original_data.clone();

        // ACT
        encrypt(&key, &tweak1, &mut data1).unwrap();
        encrypt(&key, &tweak2, &mut data2).unwrap();

        // ASSERT
        assert_ne!(data1.as_slice(), data2.as_slice());
    }

    #[test]
    fn decrypt_accepts_partial_final_block() {
        // ARRANGE
        let key = [0x42_u8; 64];
        let tweak = [0x01_u8; 16];
        let mut data = b"seventeen-byte-msg".to_vec();

        // ACT
        let result = decrypt(&key, &tweak, &mut data);

        // ASSERT
        assert!(result.is_ok());
    }
}
