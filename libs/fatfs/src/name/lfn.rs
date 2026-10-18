//! Long file name entry encoding.

use super::short::NAME_LEN;
use crate::types::ATTR_LFN;

/// Largest number of UTF-16 units an LFN may carry.
const CHAR_LIMIT: usize = 13 * 40;
/// UTF-16 units carried by one LFN fragment.
const FRAGMENT_CHARS: usize = 13;
/// Bytes in one directory entry.
const ENTRY_BYTES: usize = 32;
/// Ordinal flag marking the fragment adjacent to the 8.3 entry.
const LAST_FRAGMENT: u8 = 0x40;

/// Returns the number of LFN entries `name` needs.
pub(crate) fn fragment_count(name: &str) -> u64 {
    let chars = u64::try_from(name.encode_utf16().take(CHAR_LIMIT).count()).unwrap_or(0);
    let width = u64::try_from(FRAGMENT_CHARS).unwrap_or(1);

    chars.div_ceil(width)
}

/// Returns the LFN checksum of `alias`.
pub(crate) fn checksum(alias: &[u8; NAME_LEN]) -> u8 {
    let mut sum = 0_u8;
    for &byte in alias {
        sum = sum.rotate_right(1).wrapping_add(byte);
    }

    sum
}

/// Appends the LFN entries of `name` in on-disk order for `checksum`.
pub(crate) fn append_entries(buf: &mut Vec<u8>, name: &str, checksum: u8) {
    let mut collected = [0_u16; CHAR_LIMIT];
    let mut count = 0_usize;
    for code in name.encode_utf16().take(CHAR_LIMIT) {
        if let Some(slot) = collected.get_mut(count) {
            *slot = code;
            count = count.saturating_add(1);
        }
    }
    let chars = collected.get(..count).unwrap_or(&[]);
    let total = chars.len().div_ceil(FRAGMENT_CHARS);
    for index in (0..total).rev() {
        let start = index.saturating_mul(FRAGMENT_CHARS);
        let end = start.saturating_add(FRAGMENT_CHARS).min(chars.len());
        let chunk = chars.get(start..end).unwrap_or(&[]);
        let ordinal = if index.saturating_add(1) == total {
            LAST_FRAGMENT
        } else {
            0
        };
        let value = u8::try_from(index.saturating_add(1)).unwrap_or(0);
        buf.extend_from_slice(&fragment(ordinal | value, checksum, chunk));
    }
}

fn fragment(ordinal: u8, checksum: u8, chars: &[u16]) -> [u8; ENTRY_BYTES] {
    let mut entry = [0_u8; ENTRY_BYTES];
    if let Some(slot) = entry.first_mut() {
        *slot = ordinal;
    }
    if let Some(slot) = entry.get_mut(11) {
        *slot = ATTR_LFN;
    }
    if let Some(slot) = entry.get_mut(13) {
        *slot = checksum;
    }
    for (index, &code) in chars.iter().enumerate() {
        let offset = char_offset(index);
        if let Some(slot) = entry.get_mut(offset..offset.saturating_add(2)) {
            slot.copy_from_slice(&code.to_le_bytes());
        }
    }

    entry
}

fn char_offset(index: usize) -> usize {
    if index < 5 {
        1_usize.saturating_add(index.saturating_mul(2))
    } else if index < 11 {
        14_usize.saturating_add(index.saturating_sub(5).saturating_mul(2))
    } else {
        28_usize.saturating_add(index.saturating_sub(11).saturating_mul(2))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::name::short::ShortNames;

    fn alias(name: &str) -> [u8; NAME_LEN] {
        ShortNames::new()
            .assign(name)
            .expect("alias must be assigned")
    }

    fn fragments(name: &str) -> Vec<[u8; ENTRY_BYTES]> {
        let mut buf = Vec::new();
        append_entries(&mut buf, name, checksum(&alias(name)));

        buf.chunks(ENTRY_BYTES)
            .filter_map(|chunk| <[u8; ENTRY_BYTES]>::try_from(chunk).ok())
            .collect()
    }

    fn decode(entry: &[u8; ENTRY_BYTES]) -> String {
        [1_usize, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30]
            .into_iter()
            .map(|offset| {
                entry
                    .get(offset..offset.saturating_add(2))
                    .and_then(|bytes| bytes.try_into().ok())
                    .map_or(0, u16::from_le_bytes)
            })
            .take_while(|&code| code != 0)
            .filter(|&code| code != 0xFFFF)
            .map(|code| char::from_u32(u32::from(code)).unwrap_or_default())
            .collect()
    }

    #[test]
    fn checksum_matches_the_spec_reference_values() {
        // ARRANGE
        let cases = [("BIN", 0x7F_u8), ("BOOTX64.EFI", 0x1D_u8)];

        // ACT / ASSERT
        for (name, expected) in cases {
            assert_eq!(checksum(&alias(name)), expected, "checksum of {name}");
        }
    }

    #[test]
    fn fragment_count_covers_every_boundary() {
        // ARRANGE
        let cases = [
            (13_usize, 1_u64),
            (14, 2),
            (26, 2),
            (27, 3),
            (39, 3),
            (40, 4),
        ];

        // ACT / ASSERT
        for (chars, expected) in cases {
            let name = "a".repeat(chars);
            assert_eq!(fragment_count(&name), expected, "{chars} characters");
        }
    }

    #[test]
    fn fragments_are_ordered_from_last_to_first() {
        // ARRANGE
        let name = "File with very long filename.ext";

        // ACT
        let entries = fragments(name);

        // ASSERT
        assert_eq!(entries.len(), 3, "38 characters need three fragments");
        assert_eq!(entries.first().map(|entry| entry[0]), Some(0x43));
        assert_eq!(entries.get(1).map(|entry| entry[0]), Some(0x02));
        assert_eq!(entries.last().map(|entry| entry[0]), Some(0x01));
        for entry in &entries {
            assert_eq!(entry[11], ATTR_LFN, "attribute");
            assert_eq!(entry[13], checksum(&alias(name)), "checksum");
            assert_eq!(entry.get(26..28), Some(&[0, 0][..]), "cluster field");
        }
    }

    #[test]
    fn fragments_reconstruct_the_original_name() {
        // ARRANGE
        let names = [
            "act-led.dtbo",
            "LICENCE.broadcom",
            "File with very long filename.ext",
            "a-name-with-exactly-26-char",
        ];

        // ACT / ASSERT
        for name in names {
            let mut collected: Vec<(u8, String)> = fragments(name)
                .iter()
                .map(|entry| (entry[0] & 0x3F, decode(entry)))
                .collect();
            collected.sort_by_key(|&(ordinal, _)| ordinal);
            let rebuilt: String = collected.into_iter().map(|(_, text)| text).collect();
            assert_eq!(rebuilt, name, "round trip of {name}");
        }
    }

    #[test]
    fn fragment_count_matches_the_emitted_entries() {
        // ARRANGE
        let names = [
            "act-led.dtbo",
            "bcm2711-rpi-4-b.dtb",
            "File with very long filename.ext",
        ];

        // ACT / ASSERT
        for name in names {
            let expected = usize::try_from(fragment_count(name)).unwrap_or(0);
            assert_eq!(fragments(name).len(), expected, "fragments of {name}");
        }
    }
}
