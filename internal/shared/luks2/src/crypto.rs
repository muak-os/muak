//! AES-XTS encryption and decryption.
//!
//! Implements IEEE P1619 XTS-AES mode using the `aes` crate for the underlying
//! block cipher. XTS splits the key into two halves: one for data encryption
//! and one for tweak encryption.

use aes::Aes256;
use aes::cipher::{BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use zeroize::Zeroize;

const AES_BLOCK_SIZE: usize = 16;

/// Encrypts `data` in-place using AES-256-XTS.
///
/// `key` must be 64 bytes (two 256-bit AES keys).
/// `tweak` is the 16-byte sector/unit number in little-endian.
pub fn encrypt(key: &[u8], tweak: &[u8; 16], data: &mut [u8]) -> crate::error::Result<()> {
    xts_process(key, tweak, data, Mode::Encrypt)
}

/// Decrypts `data` in-place using AES-256-XTS.
///
/// `key` must be 64 bytes (two 256-bit AES keys).
/// `tweak` is the 16-byte sector/unit number in little-endian.
pub fn decrypt(key: &[u8], tweak: &[u8; 16], data: &mut [u8]) -> crate::error::Result<()> {
    xts_process(key, tweak, data, Mode::Decrypt)
}

enum Mode {
    Encrypt,
    Decrypt,
}

fn xts_process(
    key: &[u8],
    tweak: &[u8; 16],
    data: &mut [u8],
    mode: Mode,
) -> crate::error::Result<()> {
    if key.len() != 64 {
        return Err(crate::error::Error::InvalidField(
            "AES-XTS key must be 64 bytes".into(),
        ));
    }

    if data.len() < AES_BLOCK_SIZE {
        return Err(crate::error::Error::InvalidField(
            "data must be at least 16 bytes".into(),
        ));
    }

    let key1: &[u8; 32] = key[..32]
        .try_into()
        .map_err(|_| crate::error::Error::InvalidField("invalid AES key length".into()))?;
    let key2: &[u8; 32] = key[32..]
        .try_into()
        .map_err(|_| crate::error::Error::InvalidField("invalid AES key length".into()))?;

    let cipher1 = Aes256::new(key1.into());
    let cipher2 = Aes256::new(key2.into());

    // Encrypt the tweak with cipher2 to get the initial tweak value
    let mut t = *tweak;
    cipher2.encrypt_block((&mut t).into());

    let full_blocks = data.len() / AES_BLOCK_SIZE;

    for i in 0..full_blocks {
        let offset = i * AES_BLOCK_SIZE;
        let block = &mut data[offset..offset + AES_BLOCK_SIZE];

        // XOR with tweak
        xor_block(block, &t);

        // Encrypt or decrypt with cipher1
        let aes_block: &mut [u8; 16] = block
            .try_into()
            .map_err(|_| crate::error::Error::InvalidField("invalid block length".into()))?;
        match mode {
            Mode::Encrypt => cipher1.encrypt_block(aes_block.into()),
            Mode::Decrypt => cipher1.decrypt_block(aes_block.into()),
        }

        // XOR with tweak again
        xor_block(block, &t);

        // Advance tweak for next block
        gf128_mul_x(&mut t);
    }

    // Zeroize the tweak value
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
///
/// The reduction polynomial is x^128 + x^7 + x^2 + x + 1 (0x87).
fn gf128_mul_x(tweak: &mut [u8; 16]) {
    let mut carry = 0u8;
    for byte in tweak.iter_mut() {
        let new_carry = *byte >> 7;
        *byte = (*byte << 1) | carry;
        carry = new_carry;
    }
    // If the MSB was set, reduce with the polynomial
    tweak[0] ^= carry * 0x87;
}
