//! Helpers for encoding inline security label payloads.

use super::filter::compute_name_filter;
use crate::checked::{add, align_up, sub, u8_from_usize, u16_from_usize, write_byte, write_bytes};

pub const EROFS_XATTR_INDEX_SECURITY: u8 = 6;
pub const XATTR_HEADER_SIZE: usize = 12;
pub const XATTR_ENTRY_HEADER_SIZE: usize = 4;

/// Compute the inline xattr payload for a single `security.selinux` label.
pub fn payload(label: &[u8]) -> Vec<u8> {
    let name_suffix = b"selinux";
    let e_name_len = name_suffix.len();
    let e_value_size = label.len();

    let Some(entry_size) =
        add(XATTR_ENTRY_HEADER_SIZE, e_name_len).and_then(|size| add(size, e_value_size))
    else {
        return Vec::new();
    };
    let aligned_entry_size = align4(entry_size);
    let Some(total) = add(XATTR_HEADER_SIZE, aligned_entry_size) else {
        return Vec::new();
    };

    let mut buf = vec![0_u8; total];

    let name_filter = compute_name_filter(EROFS_XATTR_INDEX_SECURITY, name_suffix);
    let off = XATTR_HEADER_SIZE;
    let Some(name_len) = u8_from_usize(e_name_len) else {
        return Vec::new();
    };
    let Some(value_size) = u16_from_usize(e_value_size) else {
        return Vec::new();
    };
    let Some(name_offset) = add(off, XATTR_ENTRY_HEADER_SIZE) else {
        return Vec::new();
    };
    let Some(value_offset) = add(name_offset, e_name_len) else {
        return Vec::new();
    };

    let wrote_all = write_bytes(&mut buf, 0, &name_filter.to_le_bytes())
        && write_byte(&mut buf, 4, 0)
        && write_byte(&mut buf, off, name_len)
        && write_byte(&mut buf, off.saturating_add(1), EROFS_XATTR_INDEX_SECURITY)
        && write_bytes(&mut buf, off.saturating_add(2), &value_size.to_le_bytes())
        && write_bytes(&mut buf, name_offset, name_suffix)
        && write_bytes(&mut buf, value_offset, label);

    if !wrote_all {
        debug_assert!(
            wrote_all,
            "xattr buffer layout should fit the allocated payload"
        );
        return Vec::new();
    }

    buf
}

/// Compute `i_xattr_icount` from the total inline xattr payload size.
pub fn icount(payload_size: usize) -> u16 {
    if payload_size == 0 {
        return 0;
    }

    let Some(data_size) = sub(payload_size, XATTR_HEADER_SIZE) else {
        return 0;
    };
    let units = data_size.div_ceil(4).saturating_add(1);

    u16::try_from(units).unwrap_or(u16::MAX)
}

/// Round up to the next multiple of 4.
pub fn align4(val: usize) -> usize {
    align_up(val, 4).unwrap_or(val)
}

#[cfg(test)]
mod tests {
    use super::{EROFS_XATTR_INDEX_SECURITY, XATTR_HEADER_SIZE, align4, icount, payload};
    use crate::checked::{u8_from_usize, u16_from_usize};

    #[test]
    fn selinux_xattr_header_magic() {
        // ARRANGE & ACT
        let payload = payload(b"system_u:object_r:file_t:s0");

        // ASSERT
        assert!(payload.len().is_multiple_of(4));
        let filter = u32::from_le_bytes(
            payload
                .get(0..4)
                .expect("filter bytes")
                .try_into()
                .expect("4 bytes"),
        );
        assert_ne!(filter, u32::MAX);
    }

    #[test]
    fn selinux_xattr_entry_fields() {
        // ARRANGE
        let label = b"system_u:object_r:file_t:s0";

        // ACT
        let payload = payload(label);

        // ASSERT
        assert_eq!(*payload.get(XATTR_HEADER_SIZE).expect("name size byte"), 7);
        assert_eq!(
            *payload
                .get(XATTR_HEADER_SIZE + 1)
                .expect("xattr index byte"),
            EROFS_XATTR_INDEX_SECURITY
        );
        let val_size = u16::from_le_bytes(
            payload
                .get(XATTR_HEADER_SIZE + 2..XATTR_HEADER_SIZE + 4)
                .expect("value size bytes")
                .try_into()
                .expect("2 bytes"),
        );
        assert_eq!(usize::from(val_size), label.len());
    }

    #[test]
    fn xattr_icount_formula() {
        // ARRANGE & ACT
        let payload = payload(b"system_u:object_r:file_t:s0");
        let count = icount(payload.len());

        // ASSERT
        assert!(count > 0);
        let ibody_size = XATTR_HEADER_SIZE + (usize::from(count) - 1) * 4;
        assert!(ibody_size >= payload.len());
    }

    #[test]
    fn xattr_4byte_alignment() {
        // ARRANGE & ACT
        let first_payload = payload(b"x");
        let second_payload = payload(b"xx");

        // ASSERT
        assert!(first_payload.len().is_multiple_of(4));
        assert!(second_payload.len().is_multiple_of(4));
    }

    #[test]
    fn empty_xattr_produces_no_data() {
        // ARRANGE & ACT & ASSERT
        assert_eq!(icount(0), 0);
    }

    #[test]
    fn align4_edge_cases() {
        // ARRANGE & ACT & ASSERT
        assert_eq!(align4(0), 0);
        assert_eq!(align4(1), 4);
        assert_eq!(align4(2), 4);
        assert_eq!(align4(3), 4);
        assert_eq!(align4(4), 4);
        assert_eq!(align4(5), 8);
        assert_eq!(align4(6), 8);
        assert_eq!(align4(7), 8);
        assert_eq!(align4(8), 8);
        assert_eq!(align4(9), 12);
        assert_eq!(align4(10), 12);
        assert_eq!(align4(11), 12);
        assert_eq!(align4(12), 12);
    }

    #[test]
    fn build_selinux_xattr_empty_label() {
        // ARRANGE & ACT
        let payload = payload(b"");

        // ASSERT
        assert!(payload.len() >= XATTR_HEADER_SIZE + 4);
        assert!(payload.len().is_multiple_of(4));
    }

    #[test]
    fn build_selinux_xattr_one_byte_value() {
        // ARRANGE & ACT
        let payload = payload(b"x");

        // ASSERT
        assert!(payload.len() >= XATTR_HEADER_SIZE + 8);
        assert!(payload.len().is_multiple_of(4));
    }

    #[test]
    fn build_selinux_xattr_four_byte_value() {
        // ARRANGE & ACT
        let payload = payload(b"xxxx");

        // ASSERT
        assert!(payload.len() >= XATTR_HEADER_SIZE + 8);
        assert!(payload.len().is_multiple_of(4));
    }

    #[test]
    fn build_selinux_xattr_five_byte_value() {
        // ARRANGE & ACT
        let payload = payload(b"xxxxx");

        // ASSERT
        assert!(payload.len() >= XATTR_HEADER_SIZE + 12);
        assert!(payload.len().is_multiple_of(4));
    }

    #[test]
    fn xattr_icount_minimum_payload() {
        // ARRANGE & ACT
        let payload = payload(b"x");
        let count = icount(payload.len());

        // ASSERT
        assert_eq!(count, 4);
    }

    #[test]
    fn xattr_icount_with_full_payload() {
        // ARRANGE & ACT
        let payload = payload(b"system_u:object_r:admin_home_t:s0");
        let count = icount(payload.len());

        // ASSERT
        assert_eq!(count, 12);
    }

    #[test]
    fn build_selinux_xattr_returns_empty_when_value_cannot_fit() {
        // ARRANGE
        let huge_value_len = usize::from(u16::MAX).saturating_add(1);
        let huge_label = vec![b'x'; huge_value_len];

        // ACT
        let payload = payload(&huge_label);

        // ASSERT
        assert!(u8_from_usize(b"selinux".len()).is_some());
        assert!(u16_from_usize(huge_value_len).is_none());
        assert!(payload.is_empty());
    }

    #[test]
    fn xattr_icount_saturates_for_huge_payload() {
        // ARRANGE & ACT
        let count = icount(usize::MAX);

        // ASSERT
        assert_eq!(count, u16::MAX);
    }
}
