//! Unique 8.3 short-name allocation.

use std::collections::HashSet;

use crate::error::{FatError, Result};

/// Length in bytes of the 8.3 name field of a directory entry.
pub(crate) const NAME_LEN: usize = 11;

/// Capacity of the uppercase scratch buffer.
const UPPER_CAP: usize = 512;
/// Longest base name kept in the 8.3 field.
const BASE_MAX: usize = 8;
/// Longest extension kept in the 8.3 field.
const EXT_MAX: usize = 3;
/// Highest numeric tail the allocator tries before giving up.
const MAX_TAIL: u32 = 999_999;
/// Characters kept verbatim in an 8.3 name.
const VALID: &[u8] = b"_^$~!#%&-@'(){}";

/// Allocates the unique 8.3 aliases of one directory.
pub(crate) struct ShortNames {
    taken: HashSet<[u8; NAME_LEN]>,
}

impl ShortNames {
    /// Creates an allocator for an empty directory.
    pub(crate) fn new() -> Self {
        Self {
            taken: HashSet::new(),
        }
    }

    /// Returns the 8.3 alias of `name`, adding a numeric tail when required.
    ///
    /// # Errors
    ///
    /// Returns `Error::Fat` when every numeric tail is already taken.
    pub(crate) fn assign(&mut self, name: &str) -> Result<[u8; NAME_LEN]> {
        let identity = identity(name).and_then(|alias| self.take(alias));
        if let Some(alias) = identity {
            return Ok(alias);
        }
        let candidate = (1..=MAX_TAIL)
            .map(|tail| tailed(name, tail))
            .find(|alias| !self.taken.contains(alias))
            .ok_or_else(|| FatError::Fat(format!("no free 8.3 alias left for {name}")))?;
        self.taken.insert(candidate);

        Ok(candidate)
    }

    fn take(&mut self, alias: [u8; NAME_LEN]) -> Option<[u8; NAME_LEN]> {
        self.taken.insert(alias).then_some(alias)
    }
}

/// Returns whether `alias` is the plain 8.3 form of `name`.
pub(crate) fn fits(alias: &[u8; NAME_LEN], name: &str) -> bool {
    identity(name).is_some_and(|identity| identity == *alias)
}

fn identity(name: &str) -> Option<[u8; NAME_LEN]> {
    let mut upper = [0_u8; UPPER_CAP];
    let len = uppercase(name, &mut upper)?;
    let split = split(upper.get(..len)?);
    if split.root.len() > BASE_MAX || split.ext.len() > EXT_MAX {
        return None;
    }
    if split
        .root
        .iter()
        .chain(split.ext)
        .any(|&byte| sanitize(byte) != byte)
    {
        return None;
    }

    Some(field(&split, BASE_MAX))
}

fn tailed(name: &str, tail: u32) -> [u8; NAME_LEN] {
    let mut upper = [0_u8; UPPER_CAP];
    let len = uppercase(name, &mut upper).unwrap_or(UPPER_CAP);
    let split = split(upper.get(..len).unwrap_or(&upper));
    let base_max = BASE_MAX.saturating_sub(digits(tail).saturating_add(1));
    let used = split.root.len().min(base_max);
    let mut alias = field(&split, used);
    if let Some(slot) = alias.get_mut(used) {
        *slot = b'~';
    }
    write_tail(&mut alias, used, tail);

    alias
}

fn field(split: &Split<'_>, root_max: usize) -> [u8; NAME_LEN] {
    let mut alias = [b' '; NAME_LEN];
    for (slot, &byte) in alias.iter_mut().take(root_max).zip(split.root) {
        *slot = sanitize(byte);
    }
    for (slot, &byte) in alias.iter_mut().skip(BASE_MAX).zip(split.ext) {
        *slot = sanitize(byte);
    }

    alias
}

fn write_tail(alias: &mut [u8; NAME_LEN], used: usize, tail: u32) {
    let first_digit = used.saturating_add(1);
    let mut slot = first_digit.saturating_add(digits(tail));
    let mut rest = tail;
    while slot > first_digit {
        slot = slot.saturating_sub(1);
        if let Some(byte) = alias.get_mut(slot) {
            *byte = b'0'.wrapping_add(u8::try_from(rest.rem_euclid(10)).unwrap_or(0));
        }
        rest = rest.div_euclid(10);
    }
}

fn digits(value: u32) -> usize {
    let mut rest = value;
    let mut count = 1_usize;
    while rest >= 10 {
        rest = rest.div_euclid(10);
        count = count.saturating_add(1);
    }

    count
}

fn uppercase(name: &str, buf: &mut [u8; UPPER_CAP]) -> Option<usize> {
    let mut len = 0_usize;
    for upper in name.chars().flat_map(char::to_uppercase) {
        let mut scratch = [0_u8; 4];
        for &byte in upper.encode_utf8(&mut scratch).as_bytes() {
            *buf.get_mut(len)? = byte;
            len = len.saturating_add(1);
        }
    }

    Some(len)
}

fn sanitize(byte: u8) -> u8 {
    if byte.is_ascii_alphanumeric() || VALID.contains(&byte) {
        byte
    } else {
        b'_'
    }
}

struct Split<'a> {
    root: &'a [u8],
    ext: &'a [u8],
}

fn split(upper: &[u8]) -> Split<'_> {
    match upper.iter().rposition(|&byte| byte == b'.') {
        Some(dot) if dot > 0 => Split {
            root: upper.get(..dot).unwrap_or(&[]),
            ext: upper.get(dot.saturating_add(1)..).unwrap_or(&[]),
        },
        _ => Split {
            root: upper,
            ext: &[],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assigned(names: &[&str]) -> Vec<String> {
        let mut allocator = ShortNames::new();

        names
            .iter()
            .map(|name| {
                let alias = allocator.assign(name).expect("alias must be assigned");
                String::from_utf8_lossy(&alias).into_owned()
            })
            .collect()
    }

    #[test]
    fn plain_names_keep_their_identity_alias() {
        // ARRANGE
        let names = ["CONFIG.TXT", "START4.ELF", "U-BOOT.BIN", "OVERLAYS"];

        // ACT
        let aliases = assigned(&names);

        // ASSERT
        assert_eq!(
            aliases,
            vec![
                "CONFIG  TXT".to_owned(),
                "START4  ELF".to_owned(),
                "U-BOOT  BIN".to_owned(),
                "OVERLAYS   ".to_owned(),
            ]
        );
    }

    #[test]
    fn lowercase_8_3_names_keep_the_identity_alias_without_a_tail() {
        // ARRANGE
        let names = ["config.txt", "start4.elf", "u-boot.bin"];

        // ACT
        let aliases = assigned(&names);

        // ASSERT
        assert_eq!(
            aliases,
            vec![
                "CONFIG  TXT".to_owned(),
                "START4  ELF".to_owned(),
                "U-BOOT  BIN".to_owned(),
            ],
            "names that fit 8.3 must not gain an LFN entry"
        );
    }

    #[test]
    fn colliding_names_get_sequential_tails() {
        // ARRANGE
        let names: Vec<String> = (0..12)
            .map(|index| format!("bcm2711-rpi-4-b-{index}.dtb"))
            .collect();
        let borrowed: Vec<&str> = names.iter().map(String::as_str).collect();

        // ACT
        let aliases = assigned(&borrowed);

        // ASSERT
        let unique: HashSet<&String> = aliases.iter().collect();
        assert_eq!(unique.len(), names.len(), "every alias must be distinct");
        assert_eq!(aliases.first().map(String::as_str), Some("BCM271~1DTB"));
        assert_eq!(aliases.get(1).map(String::as_str), Some("BCM271~2DTB"));
        assert_eq!(aliases.get(9).map(String::as_str), Some("BCM27~10DTB"));
        assert_eq!(aliases.get(11).map(String::as_str), Some("BCM27~12DTB"));
    }

    #[test]
    fn plain_alias_colliding_with_an_assigned_tail_gets_its_own_tail() {
        // ARRANGE
        let names = ["bcm2711-rpi-4-b.dtb", "BCM271~1.DTB"];

        // ACT
        let aliases = assigned(&names);

        // ASSERT
        assert_eq!(aliases.first().map(String::as_str), Some("BCM271~1DTB"));
        assert_eq!(
            aliases.get(1).map(String::as_str),
            Some("BCM271~2DTB"),
            "the plain 8.3 name must not reuse an allocated alias"
        );
    }

    #[test]
    fn invalid_characters_are_sanitized_and_force_a_tail() {
        // ARRANGE
        let names = ["a b?.txt"];

        // ACT
        let aliases = assigned(&names);

        // ASSERT
        assert_eq!(
            aliases,
            vec!["A_B_~1  TXT".to_owned()],
            "no embedded spaces may be left before the numeric tail"
        );
    }

    #[test]
    fn leading_dot_names_are_treated_as_extensionless() {
        // ARRANGE
        let names = [".gitignore"];

        // ACT
        let aliases = assigned(&names);

        // ASSERT
        assert_eq!(aliases, vec!["_GITIG~1   ".to_owned()]);
    }

    #[test]
    fn fits_accepts_only_the_identity_form() {
        // ARRANGE
        let mut allocator = ShortNames::new();
        let identity = allocator.assign("CONFIG.TXT").expect("alias");
        let tailed = allocator.assign("config.txt").expect("alias");

        // ACT / ASSERT
        assert!(fits(&identity, "CONFIG.TXT"));
        assert!(fits(&identity, "config.txt"));
        assert!(!fits(&tailed, "config.txt"));
    }

    #[test]
    fn allocation_is_deterministic_for_the_same_order() {
        // ARRANGE
        let names = ["alpha-long-name.txt", "beta-long-name.txt", "ALPHA-L~1.TXT"];

        // ACT
        let first = assigned(&names);
        let second = assigned(&names);

        // ASSERT
        assert_eq!(first, second);
        let unique: HashSet<&String> = first.iter().collect();
        assert_eq!(unique.len(), names.len());
    }
}
