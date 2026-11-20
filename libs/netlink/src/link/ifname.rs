//! Linux network interface name validation.

use alloc::string::String;
use core::borrow::Borrow;
use core::fmt;
use core::str::FromStr;

use thiserror::Error;

/// A validated Linux network interface name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Name(String);

impl Name {
    const MAX_LEN: usize = 15;

    /// Creates a new [`Name`], validating length and content.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidName`] when the provided interface name is empty, too long, or contains a NUL byte.
    pub fn new<Source>(name: Source) -> Result<Self, InvalidName>
    where
        Source: Into<String>,
    {
        let name = name.into();
        if name.is_empty() || name.len() > Self::MAX_LEN || name.contains('\0') {
            return Err(InvalidName(name));
        }
        Ok(Self(name))
    }

    /// Returns the name as a string slice.
    #[must_use = "callers need the name to match interfaces"]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for Name {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for Name {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl FromStr for Name {
    type Err = InvalidName;

    fn from_str(source: &str) -> Result<Self, Self::Err> {
        Self::new(source.to_owned())
    }
}

impl PartialEq<str> for Name {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for Name {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// An invalid network interface name.
#[derive(Debug, Error)]
#[error("invalid interface name: {0:?}")]
pub struct InvalidName(String);

#[cfg(test)]
mod tests {
    use super::Name;

    #[test]
    fn name_rejects_empty() {
        // ACT / ASSERT
        Name::new("").unwrap_err();
    }

    #[test]
    fn name_rejects_too_long() {
        // ACT / ASSERT
        Name::new("a".repeat(16)).unwrap_err();
    }

    #[test]
    fn name_rejects_null_byte() {
        // ACT / ASSERT
        Name::new("eth\0").unwrap_err();
    }

    #[test]
    fn name_accepts_valid() {
        // ACT / ASSERT
        Name::new("eth0").unwrap();
        Name::new("enp3s0f0").unwrap();
        Name::new("a".repeat(15)).unwrap();
    }

    #[test]
    fn name_display_as_ref_borrow_and_from_str() {
        // ARRANGE
        let name: Name = "eth0".parse().expect("valid name");

        // ACT
        let display = name.to_string();
        let as_ref = name.as_ref();
        let borrowed = core::borrow::Borrow::<str>::borrow(&name);

        // ASSERT
        assert_eq!(display, "eth0");
        assert_eq!(as_ref, "eth0");
        assert_eq!(borrowed, "eth0");
    }

    #[test]
    fn name_partial_eq_str() {
        // ARRANGE
        let name = Name::new("eth0").unwrap();

        // ACT / ASSERT
        assert_eq!(name, "eth0");
        assert_ne!(name, "eth1");
    }
}
