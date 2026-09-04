//! PBKDF2-HMAC-SHA256 key derivation (RFC 8018, section 5.2).

use core::num::NonZeroU32;

use hmac::{HmacReset, KeyInit as _, Mac as _};
use sha2::Sha256;

use crate::error::Luks2Error;

type HmacSha256 = HmacReset<Sha256>;

const BLOCK_LEN: usize = 32;

/// Derives 32 bytes from `password` and `salt` using PBKDF2-HMAC-SHA256.
pub(crate) fn derive(
    password: &[u8],
    salt: &[u8],
    iterations: NonZeroU32,
) -> Result<[u8; BLOCK_LEN], Luks2Error> {
    let mut mac = HmacSha256::new_from_slice(password)
        .map_err(|_error| Luks2Error::InvalidField("HMAC key setup failed".into()))?;
    let mut block = [0_u8; BLOCK_LEN];

    // U_1 = HMAC(P, S || INT(1)); T_1 = U_1
    mac.update(salt);
    mac.update(&1_u32.to_be_bytes());
    let mut mac_out = mac.finalize_reset().into_bytes();
    block.copy_from_slice(&mac_out);

    // T_c = T_{c-1} XOR U_c
    for _ in 1..iterations.get() {
        mac.update(&mac_out);
        mac_out = mac.finalize_reset().into_bytes();
        for (byte, value) in block.iter_mut().zip(mac_out.iter()) {
            *byte ^= value;
        }
    }

    Ok(block)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn derive_hex(password: &[u8], salt: &[u8], iterations: u32) -> [u8; BLOCK_LEN] {
        derive(
            password,
            salt,
            NonZeroU32::new(iterations).expect("iterations must be non-zero"),
        )
        .expect("derive digest")
    }

    #[test]
    fn derives_rfc_vector_one_iteration() {
        // ARRANGE
        let expected = [
            0x12, 0x0f, 0xb6, 0xcf, 0xfc, 0xf8, 0xb3, 0x2c, 0x43, 0xe7, 0x22, 0x52, 0x56, 0xc4,
            0xf8, 0x37, 0xa8, 0x65, 0x48, 0xc9, 0x2c, 0xcc, 0x35, 0x48, 0x08, 0x05, 0x98, 0x7c,
            0xb7, 0x0b, 0xe1, 0x7b,
        ];

        // ACT
        let derived = derive_hex(b"password", b"salt", 1);

        // ASSERT
        assert_eq!(derived, expected);
    }

    #[test]
    fn derives_rfc_vector_two_iterations() {
        // ARRANGE
        let expected = [
            0xae, 0x4d, 0x0c, 0x95, 0xaf, 0x6b, 0x46, 0xd3, 0x2d, 0x0a, 0xdf, 0xf9, 0x28, 0xf0,
            0x6d, 0xd0, 0x2a, 0x30, 0x3f, 0x8e, 0xf3, 0xc2, 0x51, 0xdf, 0xd6, 0xe2, 0xd8, 0x5a,
            0x95, 0x47, 0x4c, 0x43,
        ];

        // ACT
        let derived = derive_hex(b"password", b"salt", 2);

        // ASSERT
        assert_eq!(derived, expected);
    }

    #[test]
    fn derives_rfc_vector_many_iterations() {
        // ARRANGE
        let expected = [
            0xc5, 0xe4, 0x78, 0xd5, 0x92, 0x88, 0xc8, 0x41, 0xaa, 0x53, 0x0d, 0xb6, 0x84, 0x5c,
            0x4c, 0x8d, 0x96, 0x28, 0x93, 0xa0, 0x01, 0xce, 0x4e, 0x11, 0xa4, 0x96, 0x38, 0x73,
            0xaa, 0x98, 0x13, 0x4a,
        ];

        // ACT
        let derived = derive_hex(b"password", b"salt", 4096);

        // ASSERT
        assert_eq!(derived, expected);
    }

    #[test]
    fn derives_with_long_password_and_salt() {
        // ARRANGE
        let expected = [
            0x6b, 0x0f, 0x2d, 0xa5, 0xcb, 0x9a, 0x76, 0xfc, 0x0d, 0x56, 0xaa, 0xfb, 0x55, 0x60,
            0x14, 0xe9, 0xd5, 0x7c, 0x83, 0x2e, 0xef, 0x81, 0x80, 0x5f, 0xf5, 0xfc, 0x45, 0xae,
            0xa2, 0xbd, 0xd0, 0xf6,
        ];

        // ACT
        let derived = derive_hex(&[b'p'; 40], &[b's'; 40], 3);

        // ASSERT
        assert_eq!(derived, expected);
    }
}
