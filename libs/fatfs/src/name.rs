//! 8.3 short-name and long-file-name encoding for directory entries.

use crate::types::{ATTR_ARCHIVE, ATTR_DIRECTORY, ATTR_LFN};

/// Length in bytes of one directory entry on disk.
pub(crate) const ENTRY_LEN: u64 = 32;

/// Uppercase buffer capacity.
const UPPER_CAP: usize = 64;

/// Appends the directory entry bytes for `name` to `buf`.
pub(crate) fn append_entry(buf: &mut Vec<u8>, name: &str, is_dir: bool, cluster: u32, size: u32) {
    let attr = if is_dir { ATTR_DIRECTORY } else { ATTR_ARCHIVE };
    let mut upper = [0_u8; UPPER_CAP];
    let split = split_uppercase(name, &mut upper);
    let short = make_short_name(&split, 0);
    if short_fits(&short, &split) {
        buf.extend_from_slice(&short_entry(&short, attr, cluster, size));
        return;
    }
    let short = make_short_name(&split, 1);
    let csum = lfn_checksum(&short);
    append_lfn_entries(buf, name, csum);
    buf.extend_from_slice(&short_entry(&short, attr, cluster, size));
}

/// Predicts the directory bytes a name occupies: one entry for a plain
/// 8.3 name, plus one entry per LFN fragment otherwise.
pub(crate) fn entry_bytes_len(name: &str) -> u64 {
    let mut upper = [0_u8; UPPER_CAP];
    let split = split_uppercase(name, &mut upper);
    if short_fits(&make_short_name(&split, 0), &split) {
        return ENTRY_LEN;
    }
    let fragments = name.encode_utf16().count().div_ceil(13);

    ENTRY_LEN
        .saturating_mul(u64::try_from(fragments).unwrap_or(u64::MAX))
        .saturating_add(ENTRY_LEN)
}

pub(crate) fn short_entry(name: &[u8; 11], attr: u8, cluster: u32, size: u32) -> [u8; 32] {
    let mut e = [0_u8; 32];
    if let Some(slot) = e.get_mut(..11) {
        slot.copy_from_slice(name);
    }
    if let Some(slot) = e.get_mut(11) {
        *slot = attr;
    }
    let hi = u16::try_from(cluster.wrapping_shr(16)).unwrap_or(0);
    let lo = u16::try_from(cluster & 0xFFFF).unwrap_or(0);
    if let Some(slot) = e.get_mut(20..22) {
        slot.copy_from_slice(&hi.to_le_bytes());
    }
    if let Some(slot) = e.get_mut(26..28) {
        slot.copy_from_slice(&lo.to_le_bytes());
    }
    if let Some(slot) = e.get_mut(28..32) {
        slot.copy_from_slice(&size.to_le_bytes());
    }

    e
}

struct Split<'a> {
    root: &'a [u8],
    ext: &'a [u8],
}

fn split_uppercase<'a>(name: &str, buf: &'a mut [u8; UPPER_CAP]) -> Split<'a> {
    let mut len = 0_usize;
    for ch in name.chars() {
        if !push_uppercase(ch, buf, &mut len) {
            break;
        }
    }
    let upper = buf.get(..len).unwrap_or(&[]);
    let (root, ext) = match upper.iter().rposition(|&byte| byte == b'.') {
        Some(dot_pos) => (
            upper.get(..dot_pos).unwrap_or(&[]),
            upper.get(dot_pos.wrapping_add(1)..).unwrap_or(&[]),
        ),
        None => (upper, &[][..]),
    };

    Split { root, ext }
}

fn push_uppercase(ch: char, buf: &mut [u8], len: &mut usize) -> bool {
    let mut scratch = [0_u8; 4];
    let mut fits = true;
    for up in ch.to_uppercase() {
        for &byte in up.encode_utf8(&mut scratch).as_bytes() {
            fits &= write_upper_byte(buf, len, byte);
        }
    }

    fits
}

fn write_upper_byte(buf: &mut [u8], len: &mut usize, byte: u8) -> bool {
    let Some(slot) = buf.get_mut(*len) else {
        return false;
    };
    *slot = byte;
    *len = len.saturating_add(1);

    true
}

fn make_short_name(split: &Split<'_>, seq: u8) -> [u8; 11] {
    let tilde = seq > 0;
    let base_max: usize = if tilde { 6 } else { 8 };
    let mut sn = [b' '; 11];
    for (i, &byte) in split.root.iter().enumerate().take(base_max) {
        if let Some(slot) = sn.get_mut(i) {
            *slot = valid_char(byte);
        }
    }
    if tilde {
        if let Some(slot) = sn.get_mut(6) {
            *slot = b'~';
        }
        if let Some(slot) = sn.get_mut(7) {
            *slot = b'0'.wrapping_add(seq.min(9));
        }
    }
    for (i, &byte) in split.ext.iter().enumerate().take(3) {
        if let Some(slot) = sn.get_mut(8_usize.wrapping_add(i)) {
            *slot = valid_char(byte);
        }
    }

    sn
}

fn short_fits(short: &[u8; 11], split: &Split<'_>) -> bool {
    if split.root.len() > 8 || split.ext.len() > 3 {
        return false;
    }
    for (i, &byte) in split.root.iter().enumerate() {
        match short.get(i) {
            Some(&short_byte) if short_byte != valid_char(byte) => return false,
            None => return false,
            _ => {}
        }
    }
    for (i, &byte) in split.ext.iter().enumerate() {
        match short.get(8_usize.wrapping_add(i)) {
            Some(&short_byte) if short_byte != valid_char(byte) => return false,
            None => return false,
            _ => {}
        }
    }

    true
}

fn valid_char(byte: u8) -> u8 {
    if byte.is_ascii_alphanumeric() || b"_^$~!#%&-@'(){}".contains(&byte) {
        byte
    } else {
        b'_'
    }
}

fn lfn_checksum(short: &[u8; 11]) -> u8 {
    let mut sum = 0_u8;
    for &byte in short {
        sum = sum.rotate_right(1).wrapping_add(byte);
    }

    sum
}

fn append_lfn_entries(buf: &mut Vec<u8>, name: &str, checksum: u8) {
    let mut collected = [0_u16; LFN_CHAR_LIMIT];
    let count = name
        .encode_utf16()
        .take(LFN_CHAR_LIMIT)
        .zip(collected.iter_mut())
        .map(|(code, slot)| *slot = code)
        .count();
    let chars = collected.get(..count).unwrap_or(&[]);
    let total = chars.len().div_ceil(13);
    for i in (0..total).rev() {
        let mut ent = [0_u8; 32];
        let ordinal = if i.wrapping_add(1) == total { 0x40 } else { 0 };
        let ordinal_val = u8::try_from(i.wrapping_add(1)).unwrap_or(0);
        if let Some(slot) = ent.get_mut(0) {
            *slot = ordinal | ordinal_val;
        }
        if let Some(slot) = ent.get_mut(11) {
            *slot = ATTR_LFN;
        }
        if let Some(slot) = ent.get_mut(13) {
            *slot = checksum;
        }
        let start = i.wrapping_mul(13);
        let end = start.saturating_add(13).min(chars.len());
        let chunk = chars.get(start..end).unwrap_or(&[]);
        write_lfn_chars(&mut ent, chunk);
        buf.extend_from_slice(&ent);
    }
}

fn write_lfn_chars(ent: &mut [u8; 32], chars: &[u16]) {
    for (j, &cp) in chars.iter().enumerate() {
        let off = lfn_offset(j);
        let end = off.wrapping_add(2);
        if let Some(slot) = ent.get_mut(off..end) {
            slot.copy_from_slice(&cp.to_le_bytes());
        }
    }
}

fn lfn_offset(index: usize) -> usize {
    if index < 5 {
        1_usize.wrapping_add(index.wrapping_mul(2))
    } else if index < 11 {
        14_usize.wrapping_add(index.wrapping_sub(5).wrapping_mul(2))
    } else {
        28_usize.wrapping_add(index.wrapping_sub(11).wrapping_mul(2))
    }
}

const LFN_CHAR_LIMIT: usize = 13 * 40;

#[cfg(test)]
mod tests {
    use super::*;

    fn collected_lfn_entries(name: &str, checksum: u8) -> Vec<[u8; 32]> {
        let mut buf = Vec::new();
        append_lfn_entries(&mut buf, name, checksum);
        buf.chunks(32)
            .filter_map(|chunk| <[u8; 32]>::try_from(chunk).ok())
            .collect()
    }

    fn decode_lfn_fragment(entry: &[u8; 32]) -> String {
        [1_usize, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30]
            .into_iter()
            .map(|off| {
                entry
                    .get(off..off.wrapping_add(2))
                    .and_then(|bytes| bytes.try_into().ok())
                    .map_or(0, u16::from_le_bytes)
            })
            .take_while(|&code| code != 0)
            .filter(|&code| code != 0xFFFF)
            .map(|code| char::from_u32(u32::from(code)).unwrap_or_default())
            .collect()
    }

    fn decode_lfn_entry(entry: &[u8; 32], expected_checksum: u8) -> (u8, String) {
        assert_eq!(entry[11], ATTR_LFN, "LFN attribute");
        assert_eq!(
            entry[13], expected_checksum,
            "stored checksum must match the short name"
        );
        assert_eq!(
            entry.get(26..28),
            Some(&[0, 0][..]),
            "cluster field must be zero"
        );

        (entry[0] & 0x3F, decode_lfn_fragment(entry))
    }

    #[test]
    fn short_name_truncation() {
        // ARRANGE
        let mut upper = [0_u8; UPPER_CAP];
        let split = split_uppercase("VeryLong.Extra", &mut upper);

        // ACT
        let sn = make_short_name(&split, 1);

        // ASSERT
        assert_eq!(sn.get(..6), Some(&b"VERYLO"[..]));
        assert_eq!(sn.get(6), Some(&b'~'));
        assert_eq!(sn.get(7), Some(&b'1'));
        assert_eq!(sn.get(8..11), Some(&b"EXT"[..]));
    }

    #[test]
    fn short_name_no_tilde() {
        // ARRANGE
        let mut upper = [0_u8; UPPER_CAP];
        let split = split_uppercase("BOOTX64.EFI", &mut upper);

        // ACT
        let sn = make_short_name(&split, 0);

        // ASSERT
        assert_eq!(sn.get(..8), Some(&b"BOOTX64 "[..]));
        assert_eq!(sn.get(8..11), Some(&b"EFI"[..]));
    }

    #[test]
    fn entry_len_matches_entry_bytes_for_plain_names() {
        // ARRANGE
        let names = ["BOOTX64.EFI", "bin", "cfg.txt", "a"];

        // ACT / ASSERT
        for name in names {
            let mut buf = Vec::new();
            append_entry(&mut buf, name, false, 2, 0);
            assert_eq!(
                u64::try_from(buf.len()).unwrap_or(u64::MAX),
                entry_bytes_len(name),
                "entry length must match encoded bytes for {name}"
            );
        }
    }

    #[test]
    fn entry_len_matches_entry_bytes_for_lfn_names() {
        // ARRANGE
        let name = "File with very long filename.ext";

        // ACT
        let mut buf = Vec::new();
        append_entry(&mut buf, name, false, 2, 0);

        // ASSERT
        assert_eq!(
            u64::try_from(buf.len()).unwrap_or(u64::MAX),
            entry_bytes_len(name),
            "entry length must match encoded bytes"
        );
    }

    #[test]
    fn lfn_checksum_matches_spec_reference() {
        // ARRANGE
        let mut upper_bin = [0_u8; UPPER_CAP];
        let mut upper_boot = [0_u8; UPPER_CAP];
        let short_bin = make_short_name(&split_uppercase("bin", &mut upper_bin), 0);
        let short_boot = make_short_name(&split_uppercase("BOOTX64.EFI", &mut upper_boot), 0);

        // ACT
        let csum_bin = lfn_checksum(&short_bin);
        let csum_boot = lfn_checksum(&short_boot);

        // ASSERT
        assert_eq!(
            csum_bin, 0x7F,
            "checksum of 'BIN' padded to 11 must match the spec"
        );
        assert_eq!(
            csum_boot, 0x1D,
            "checksum of 'BOOTX64.EFI' must match the spec"
        );
    }

    #[test]
    fn lfn_entries_reconstruct_original_name() {
        // ARRANGE
        let names = [
            "act-led.dtbo",
            "LICENCE.broadcom",
            "File with very long filename.ext",
        ];

        // ACT
        for name in names {
            let mut upper = [0_u8; UPPER_CAP];
            let split = split_uppercase(name, &mut upper);
            let short = make_short_name(&split, 1);
            let csum = lfn_checksum(&short);
            let entries = collected_lfn_entries(name, csum);

            let mut fragments: Vec<(u8, String)> = entries
                .iter()
                .map(|entry| decode_lfn_entry(entry, csum))
                .collect();

            assert_eq!(
                fragments.last().map(|&(seq, _)| seq),
                Some(1),
                "seq 1 must be the fragment adjacent to the 8.3 entry"
            );

            // ASSERT
            fragments.sort_by_key(|&(seq, _)| seq);
            let reconstructed: String = fragments.into_iter().map(|(_, frag)| frag).collect();
            assert_eq!(reconstructed, name, "round-trip must reproduce the name");
        }
    }
}
