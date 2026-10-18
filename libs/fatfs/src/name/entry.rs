//! 32-byte FAT directory entry encoding.

use super::{lfn, short};
use crate::types::{ATTR_DIRECTORY, ATTR_VOLUME_ID};

/// Length in bytes of one directory entry on disk.
pub(crate) const ENTRY_LEN: u64 = 32;

/// Bytes in one directory entry.
const ENTRY_BYTES: usize = 32;
/// Bytes in the `.` and `..` entry pair.
const DOT_BYTES: usize = 64;
/// Fixed creation, access and write date: 1980-01-01, the FAT epoch.
const FIXED_DATE: u16 = 0x0021;
/// Fixed creation, access and write time: 00:00:00.
const FIXED_TIME: u16 = 0;

/// Appends the directory entry bytes of `name` using its allocated 8.3 alias.
pub(crate) fn append_entry(
    buf: &mut Vec<u8>,
    name: &str,
    alias: &[u8; short::NAME_LEN],
    attr: u8,
    cluster: u32,
    size: u32,
) {
    if !short::fits(alias, name) {
        lfn::append_entries(buf, name, lfn::checksum(alias));
    }
    buf.extend_from_slice(&entry(alias, attr, cluster, size));
}

/// Predicts the number of bytes `append_entry` emits for `name`.
pub(crate) fn entry_bytes_len(name: &str, alias: &[u8; short::NAME_LEN]) -> u64 {
    if short::fits(alias, name) {
        return ENTRY_LEN;
    }

    ENTRY_LEN.saturating_mul(lfn::fragment_count(name).saturating_add(1))
}

/// Encodes the `.` and `..` entries of a directory.
pub(crate) fn dot_entries(me: u32, parent: u32) -> [u8; DOT_BYTES] {
    let mut entries = [0_u8; DOT_BYTES];
    if let Some(slot) = entries.get_mut(..ENTRY_BYTES) {
        slot.copy_from_slice(&entry(&dotted(b"."), ATTR_DIRECTORY, me, 0));
    }
    if let Some(slot) = entries.get_mut(ENTRY_BYTES..) {
        slot.copy_from_slice(&entry(&dotted(b".."), ATTR_DIRECTORY, parent, 0));
    }

    entries
}

/// Encodes the volume label entry of the root directory.
pub(crate) fn label_entry(label: &[u8; short::NAME_LEN]) -> [u8; ENTRY_BYTES] {
    entry(label, ATTR_VOLUME_ID, 0, 0)
}

fn entry(name: &[u8; short::NAME_LEN], attr: u8, cluster: u32, size: u32) -> [u8; ENTRY_BYTES] {
    let mut encoded = [0_u8; ENTRY_BYTES];
    if let Some(slot) = encoded.get_mut(..short::NAME_LEN) {
        slot.copy_from_slice(name);
    }
    if let Some(slot) = encoded.get_mut(11) {
        *slot = attr;
    }
    put_u16(&mut encoded, 14, FIXED_TIME);
    put_u16(&mut encoded, 16, FIXED_DATE);
    put_u16(&mut encoded, 18, FIXED_DATE);
    put_u16(
        &mut encoded,
        20,
        u16::try_from(cluster.wrapping_shr(16)).unwrap_or(0),
    );
    put_u16(&mut encoded, 22, FIXED_TIME);
    put_u16(&mut encoded, 24, FIXED_DATE);
    put_u16(
        &mut encoded,
        26,
        u16::try_from(cluster & 0xFFFF).unwrap_or(0),
    );
    put_u32(&mut encoded, 28, size);

    encoded
}

fn put_u16(buf: &mut [u8; ENTRY_BYTES], offset: usize, value: u16) {
    if let Some(slot) = buf.get_mut(offset..offset.saturating_add(2)) {
        slot.copy_from_slice(&value.to_le_bytes());
    }
}

fn put_u32(buf: &mut [u8; ENTRY_BYTES], offset: usize, value: u32) {
    if let Some(slot) = buf.get_mut(offset..offset.saturating_add(4)) {
        slot.copy_from_slice(&value.to_le_bytes());
    }
}

fn dotted(name: &[u8]) -> [u8; short::NAME_LEN] {
    let mut alias = [b' '; short::NAME_LEN];
    for (slot, &byte) in alias.iter_mut().zip(name) {
        *slot = byte;
    }

    alias
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::name::short::ShortNames;
    use crate::types::ATTR_ARCHIVE;

    fn alias(name: &str) -> [u8; short::NAME_LEN] {
        ShortNames::new()
            .assign(name)
            .expect("alias must be assigned")
    }

    fn read_u16(buf: &[u8], offset: usize) -> u16 {
        buf.get(offset..offset.saturating_add(2))
            .and_then(|bytes| bytes.try_into().ok())
            .map_or(0, u16::from_le_bytes)
    }

    fn read_u32(buf: &[u8], offset: usize) -> u32 {
        buf.get(offset..offset.saturating_add(4))
            .and_then(|bytes| bytes.try_into().ok())
            .map_or(0, u32::from_le_bytes)
    }

    #[test]
    fn short_entries_carry_a_valid_timestamp() {
        // ARRANGE
        let encoded = entry(&alias("CONFIG.TXT"), ATTR_ARCHIVE, 0, 0);

        // ACT
        let dates = [
            read_u16(&encoded, 16),
            read_u16(&encoded, 18),
            read_u16(&encoded, 24),
        ];
        let times = [read_u16(&encoded, 14), read_u16(&encoded, 22)];

        // ASSERT
        assert_eq!(dates, [FIXED_DATE; 3], "creation, access and write dates");
        assert_eq!(times, [FIXED_TIME; 2], "creation and write times");
        for date in dates {
            assert_eq!((date >> 5) & 0xF, 1, "month must be January");
            assert_eq!(date & 0x1F, 1, "day must be the first");
        }
    }

    #[test]
    fn short_entries_split_high_cluster_bits() {
        // ARRANGE
        let cluster = 0x0001_2345_u32;

        // ACT
        let encoded = entry(&alias("a.bin"), ATTR_ARCHIVE, cluster, 7);

        // ASSERT
        assert_eq!(read_u16(&encoded, 20), 0x0001, "first cluster high word");
        assert_eq!(read_u16(&encoded, 26), 0x2345, "first cluster low word");
        assert_eq!(read_u32(&encoded, 28), 7, "file size");
    }

    #[test]
    fn dot_entries_point_at_the_directory_and_its_parent() {
        // ARRANGE
        let entries = dot_entries(9, 2);

        // ACT
        let (dot, dotdot) = entries.split_at(ENTRY_BYTES);

        // ASSERT
        assert_eq!(dot.get(..3), Some(&b".  "[..]), "`.` name");
        assert_eq!(dotdot.get(..3), Some(&b".. "[..]), "`..` name");
        assert_eq!(dot.get(11), Some(&ATTR_DIRECTORY), "`.` attribute");
        assert_eq!(dotdot.get(11), Some(&ATTR_DIRECTORY), "`..` attribute");
        assert_eq!(read_u16(dot, 26), 9, "`.` points at itself");
        assert_eq!(read_u16(dotdot, 26), 2, "`..` points at the parent");
    }

    #[test]
    fn label_entries_use_the_volume_id_attribute() {
        // ARRANGE
        let label = crate::types::VOLUME_LABEL;

        // ACT
        let encoded = label_entry(&label);

        // ASSERT
        assert_eq!(encoded.get(..11), Some(&label[..]), "label bytes");
        assert_eq!(encoded.get(11), Some(&ATTR_VOLUME_ID), "attribute");
        assert_eq!(read_u32(&encoded, 28), 0, "no size");
        assert_eq!(read_u16(&encoded, 26), 0, "no cluster");
    }

    #[test]
    fn entry_len_matches_the_encoded_bytes() {
        // ARRANGE
        let cases = [
            ("CONFIG.TXT", false),
            ("config.txt", false),
            ("act-led.dtbo", true),
            ("File with very long filename.ext", true),
        ];

        // ACT / ASSERT
        for (name, wants_lfn) in cases {
            let short = alias(name);
            let mut buf = Vec::new();
            append_entry(&mut buf, name, &short, ATTR_ARCHIVE, 2, 0);
            assert_eq!(
                u64::try_from(buf.len()).unwrap_or(u64::MAX),
                entry_bytes_len(name, &short),
                "length of {name}"
            );
            assert_eq!(buf.len() > ENTRY_BYTES, wants_lfn, "LFN of {name}");
        }
    }
}
