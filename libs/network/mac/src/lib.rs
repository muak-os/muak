//! IEEE 802 MAC address type, formatting and generation.

#![warn(missing_docs)]

use core::fmt;

#[cfg(feature = "generate")]
use sha2::{Digest as _, Sha256};

/// A 6-byte hardware address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Mac([u8; 6]);

impl Mac {
    /// Creates a hardware address from its raw bytes.
    #[must_use = "the address must be stored or forwarded"]
    pub const fn new(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }

    /// Returns the raw hardware address bytes.
    #[must_use = "callers need the bytes to compare links"]
    pub const fn as_bytes(&self) -> &[u8; 6] {
        &self.0
    }

    /// Consumes the address and returns its raw bytes.
    #[must_use = "callers need the bytes to compare links"]
    pub const fn into_array(self) -> [u8; 6] {
        self.0
    }
}

impl From<[u8; 6]> for Mac {
    fn from(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }
}

impl AsRef<[u8; 6]> for Mac {
    fn as_ref(&self) -> &[u8; 6] {
        &self.0
    }
}

impl fmt::Display for Mac {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut separator = "";
        for byte in self.0 {
            write!(f, "{separator}{byte:02x}")?;
            separator = ":";
        }

        Ok(())
    }
}

/// Generates a deterministic locally-administered unicast MAC from an identifier.
#[cfg(feature = "generate")]
#[must_use = "callers need the address to configure the link"]
pub fn generate(id: &str) -> Mac {
    let result = Sha256::digest(id.as_bytes());

    let mut mac = [0_u8; 6];
    for (dst, src) in mac.iter_mut().zip(result.iter()) {
        *dst = *src;
    }

    mac[0] = (mac[0] & 0xfe) | 0x02;

    Mac(mac)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_and_accessors_round_trip() {
        // ARRANGE
        let bytes = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55];

        // ACT
        let mac = Mac::new(bytes);

        // ASSERT
        assert_eq!(mac.as_bytes(), &bytes);
        assert_eq!(mac.into_array(), bytes);
    }

    #[test]
    fn from_and_as_ref() {
        // ARRANGE
        let bytes = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55];

        // ACT
        let mac = Mac::from(bytes);

        // ASSERT
        assert_eq!(mac.as_ref(), &bytes);
    }

    #[test]
    fn default_is_zero() {
        // ACT
        let mac = Mac::default();

        // ASSERT
        assert_eq!(mac.into_array(), [0; 6]);
    }

    #[test]
    fn display_is_colon_separated_lowercase() {
        // ARRANGE
        let mac = Mac::new([0x0a, 0x1b, 0x2c, 0x3d, 0x4e, 0x5f]);

        // ACT
        let display = mac.to_string();

        // ASSERT
        assert_eq!(display, "0a:1b:2c:3d:4e:5f");
    }

    #[test]
    fn display_pads_zero_bytes() {
        // ARRANGE
        let mac = Mac::new([0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);

        // ACT
        let display = mac.to_string();

        // ASSERT
        assert_eq!(display, "00:00:00:00:00:00");
    }

    #[test]
    fn display_renders_every_bit() {
        // ARRANGE
        let mac = Mac::new([0xff; 6]);

        // ACT
        let display = mac.to_string();

        // ASSERT
        assert_eq!(display, "ff:ff:ff:ff:ff:ff");
    }
}
