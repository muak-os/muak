//! Hardware address accessors and formatting.

use core::fmt;

use crate::Mac;

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

#[cfg(test)]
mod tests {
    use crate::Mac;

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
        // ACT / ASSERT
        assert_eq!(Mac::default().into_array(), [0; 6]);
    }

    #[test]
    fn display_is_colon_separated_lowercase() {
        // ARRANGE
        let mac = Mac::new([0x0a, 0x1b, 0x2c, 0x3d, 0x4e, 0x5f]);

        // ACT
        let display = mac.to_string();
        let expected = mac::format(&mac.into_array());

        // ASSERT
        assert_eq!(display, "0a:1b:2c:3d:4e:5f");
        assert_eq!(display, expected);
    }
}
