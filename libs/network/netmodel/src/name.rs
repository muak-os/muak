//! Interface name validation and string conversions.

use core::borrow::Borrow;
use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::str::FromStr;

use crate::error::NameError;
use crate::{MAX_LEN, Name};

impl Name {
    /// Creates a new [`Name`], validating length and content.
    ///
    /// # Errors
    ///
    /// Returns [`NameError`] when the provided interface name is empty, too long, or contains a NUL byte.
    pub fn new<Source>(source: Source) -> Result<Self, NameError>
    where
        Source: AsRef<str>,
    {
        let source = source.as_ref();
        if source.is_empty() {
            return Err(NameError::Empty);
        }
        if source.len() > MAX_LEN {
            return Err(NameError::TooLong);
        }
        if source.as_bytes().contains(&0) {
            return Err(NameError::Nul);
        }

        let Ok(len) = u8::try_from(source.len()) else {
            return Err(NameError::TooLong);
        };
        let mut bytes = [0_u8; MAX_LEN];
        let Some(target) = bytes.get_mut(..source.len()) else {
            return Err(NameError::TooLong);
        };
        target.copy_from_slice(source.as_bytes());

        Ok(Self { bytes, len })
    }

    /// Returns the name as a string slice.
    #[must_use = "callers need the name to match interfaces"]
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(self.bytes.get(..usize::from(self.len)).unwrap_or_default())
            .unwrap_or_default()
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Name").field(&self.as_str()).finish()
    }
}

impl AsRef<str> for Name {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for Name {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl FromStr for Name {
    type Err = NameError;

    fn from_str(source: &str) -> Result<Self, Self::Err> {
        Self::new(source)
    }
}

impl Hash for Name {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl PartialOrd for Name {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Name {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl PartialEq<str> for Name {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Name {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

#[cfg(test)]
mod tests {
    use core::borrow::Borrow;
    use core::mem::size_of;
    use std::collections::HashMap;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash as _, Hasher as _};

    use crate::Name;
    use crate::error::NameError;

    #[test]
    fn name_rejects_empty() {
        // ARRANGE
        let source = "";

        // ACT
        let error = Name::new(source).unwrap_err();

        // ASSERT
        assert_eq!(error, NameError::Empty);
    }

    #[test]
    fn name_rejects_too_long() {
        // ARRANGE
        let source = "a".repeat(16);

        // ACT
        let error = Name::new(source).unwrap_err();

        // ASSERT
        assert_eq!(error, NameError::TooLong);
    }

    #[test]
    fn name_rejects_null_byte() {
        // ARRANGE
        let source = "eth\0";

        // ACT
        let error = Name::new(source).unwrap_err();

        // ASSERT
        assert_eq!(error, NameError::Nul);
    }

    #[test]
    fn name_accepts_valid() {
        // ARRANGE
        let boundary = "a".repeat(15);

        // ACT
        let short = Name::new("eth0");
        let long = Name::new("enp3s0f0");
        let at_boundary = Name::new(boundary);

        // ASSERT
        short.unwrap();
        long.unwrap();
        at_boundary.unwrap();
    }

    #[test]
    fn name_display_as_ref_borrow_and_from_str() {
        // ARRANGE
        let name: Name = "eth0".parse().expect("valid name");

        // ACT
        let display = name.to_string();
        let as_ref = name.as_ref();
        let borrowed = Borrow::<str>::borrow(&name);

        // ASSERT
        assert_eq!(display, "eth0");
        assert_eq!(as_ref, "eth0");
        assert_eq!(borrowed, "eth0");
    }

    #[test]
    fn name_partial_eq_str() {
        // ARRANGE
        let name = Name::new("eth0").unwrap();

        // ACT
        let equal = name == "eth0";
        let different = name == "eth1";

        // ASSERT
        assert!(equal);
        assert!(!different);
    }

    #[test]
    fn name_hashes_like_its_str_view() {
        // ARRANGE
        let name = Name::new("eth0").unwrap();
        let mut map: HashMap<Name, u8> = HashMap::new();

        // ACT
        map.insert(name, 1);
        let hit = map.get("eth0");
        let mut name_hasher = DefaultHasher::new();
        name.hash(&mut name_hasher);
        let mut str_hasher = DefaultHasher::new();
        name.as_str().hash(&mut str_hasher);

        // ASSERT
        assert_eq!(name_hasher.finish(), str_hasher.finish());
        assert_eq!(hit, Some(&1));
    }

    #[test]
    fn name_ordering_matches_str() {
        // ARRANGE
        let pairs = [("eth0", "eth00"), ("eth0", "eth1"), ("ab", "abc")];

        // ACT
        let orderings: Vec<_> = pairs
            .iter()
            .map(|&(left, right)| {
                let left_name = Name::new(left).unwrap();
                let right_name = Name::new(right).unwrap();
                left_name.cmp(&right_name)
            })
            .collect();

        // ASSERT
        for (&(left, right), ordering) in pairs.iter().zip(orderings) {
            assert_eq!(ordering, left.cmp(right));
        }
    }

    #[test]
    fn name_debug_matches_the_string_shape() {
        // ARRANGE
        let name = Name::new("eth0").expect("valid name");

        // ACT
        let debug = format!("{name:?}");

        // ASSERT
        assert_eq!(debug, "Name(\"eth0\")");
    }

    #[test]
    fn name_is_sixteen_bytes_and_copy() {
        // ARRANGE
        let name = Name::new("eth0").unwrap();

        // ACT
        let moved = name;
        let size = size_of::<Name>();

        // ASSERT
        assert_eq!(size, 16);
        assert_eq!(moved, name);
    }
}
