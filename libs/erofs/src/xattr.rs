//! Inline xattr encoding for security.selinux labels.

pub const EROFS_XATTR_INDEX_SECURITY: u8 = 6;
pub const XATTR_HEADER_SIZE: usize = 12;
pub const XATTR_ENTRY_HEADER_SIZE: usize = 4;
const XATTR_FILTER_SEED: u32 = 0x25BB_E08F;
const XATTR_FILTER_DEFAULT: u32 = u32::MAX;
const XATTR_FILTER_BITS: u32 = 32;

/// Compute the inline xattr payload for a single `security.selinux` label.
pub fn build_selinux_xattr(label: &[u8]) -> Vec<u8> {
    let name_suffix = b"selinux";
    let e_name_len = name_suffix.len();
    let e_value_size = label.len();

    let entry_size = XATTR_ENTRY_HEADER_SIZE + e_name_len + e_value_size;
    let aligned_entry_size = align4(entry_size);
    let total = XATTR_HEADER_SIZE + aligned_entry_size;

    let mut buf = vec![0u8; total];

    let name_filter = compute_name_filter(EROFS_XATTR_INDEX_SECURITY, name_suffix);
    buf[0..4].copy_from_slice(&name_filter.to_le_bytes());
    buf[4] = 0;

    let off = XATTR_HEADER_SIZE;
    buf[off] = e_name_len as u8;
    buf[off + 1] = EROFS_XATTR_INDEX_SECURITY;
    buf[off + 2..off + 4].copy_from_slice(&(e_value_size as u16).to_le_bytes());
    buf[off + 4..off + 4 + e_name_len].copy_from_slice(name_suffix);
    buf[off + 4 + e_name_len..off + 4 + e_name_len + e_value_size].copy_from_slice(label);

    buf
}

/// Compute `i_xattr_icount` from the total inline xattr payload size.
pub fn xattr_icount(payload_size: usize) -> u16 {
    if payload_size == 0 {
        return 0;
    }
    let units = (payload_size - XATTR_HEADER_SIZE).div_ceil(4) + 1;
    units as u16
}

/// Compute the xattr name filter bloom-bit for an xattr.
fn compute_name_filter(base_index: u8, name_suffix: &[u8]) -> u32 {
    let hash = xxhash32(name_suffix, XATTR_FILTER_SEED + base_index as u32);
    let bit = hash & (XATTR_FILTER_BITS - 1);
    XATTR_FILTER_DEFAULT & !(1u32 << bit)
}

/// Minimal xxHash32 implementation for xattr name filter.
fn xxhash32(input: &[u8], seed: u32) -> u32 {
    const PRIME1: u32 = 0x9E37_79B1;
    const PRIME2: u32 = 0x85EB_CA77;
    const PRIME3: u32 = 0xC2B2_AE3D;
    const PRIME4: u32 = 0x27D4_EB2F;
    const PRIME5: u32 = 0x1656_67B1;

    let len = input.len() as u32;
    let mut h: u32;
    let mut i = 0usize;

    if input.len() >= 16 {
        let mut v1 = seed.wrapping_add(PRIME1).wrapping_add(PRIME2);
        let mut v2 = seed.wrapping_add(PRIME2);
        let mut v3 = seed;
        let mut v4 = seed.wrapping_sub(PRIME1);

        while i + 16 <= input.len() {
            v1 = xxh32_round(v1, read_u32_le(input, i));
            v2 = xxh32_round(v2, read_u32_le(input, i + 4));
            v3 = xxh32_round(v3, read_u32_le(input, i + 8));
            v4 = xxh32_round(v4, read_u32_le(input, i + 12));
            i += 16;
        }
        h = v1
            .rotate_left(1)
            .wrapping_add(v2.rotate_left(7))
            .wrapping_add(v3.rotate_left(12))
            .wrapping_add(v4.rotate_left(18));
    } else {
        h = seed.wrapping_add(PRIME5);
    }

    h = h.wrapping_add(len);

    while i + 4 <= input.len() {
        h = h.wrapping_add(read_u32_le(input, i).wrapping_mul(PRIME3));
        h = h.rotate_left(17).wrapping_mul(PRIME4);
        i += 4;
    }

    while i < input.len() {
        h = h.wrapping_add((input[i] as u32).wrapping_mul(PRIME5));
        h = h.rotate_left(11).wrapping_mul(PRIME1);
        i += 1;
    }

    h ^= h >> 15;
    h = h.wrapping_mul(PRIME2);
    h ^= h >> 13;
    h = h.wrapping_mul(PRIME3);
    h ^= h >> 16;
    h
}

fn xxh32_round(acc: u32, input: u32) -> u32 {
    acc.wrapping_add(input.wrapping_mul(0x85EB_CA77))
        .rotate_left(13)
        .wrapping_mul(0x9E37_79B1)
}

fn read_u32_le(buf: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        buf[offset..offset + 4]
            .try_into()
            .expect("read_u32_le: slice must be 4 bytes"),
    )
}

/// Round up to the next multiple of 4.
pub fn align4(val: usize) -> usize {
    (val + 3) & !3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selinux_xattr_header_magic() {
        // ACT
        let payload = build_selinux_xattr(b"system_u:object_r:file_t:s0");

        // ASSERT
        assert_eq!(payload.len() % 4, 0);
        let filter = u32::from_le_bytes(payload[0..4].try_into().expect("4 bytes"));
        assert_ne!(filter, u32::MAX);
    }

    #[test]
    fn selinux_xattr_entry_fields() {
        // ARRANGE
        let label = b"system_u:object_r:file_t:s0";

        // ACT
        let payload = build_selinux_xattr(label);

        // ASSERT
        assert_eq!(payload[XATTR_HEADER_SIZE], 7);
        assert_eq!(payload[XATTR_HEADER_SIZE + 1], EROFS_XATTR_INDEX_SECURITY);
        let val_size = u16::from_le_bytes(
            payload[XATTR_HEADER_SIZE + 2..XATTR_HEADER_SIZE + 4]
                .try_into()
                .expect("2 bytes"),
        );
        assert_eq!(val_size as usize, label.len());
    }

    #[test]
    fn xattr_icount_formula() {
        // ACT & ASSERT
        assert_eq!(xattr_icount(0), 0);
        let payload = build_selinux_xattr(b"system_u:object_r:file_t:s0");
        let count = xattr_icount(payload.len());
        assert!(count > 0);
        let ibody_size = XATTR_HEADER_SIZE + (count as usize - 1) * 4;
        assert!(ibody_size >= payload.len());
    }

    #[test]
    fn xattr_4byte_alignment() {
        // ACT
        let payload = build_selinux_xattr(b"x");
        let payload2 = build_selinux_xattr(b"xx");

        // ASSERT
        assert_eq!(payload.len() % 4, 0);
        assert_eq!(payload2.len() % 4, 0);
    }

    #[test]
    fn empty_xattr_produces_no_data() {
        // ACT & ASSERT
        assert_eq!(xattr_icount(0), 0);
    }

    #[test]
    fn align4_edge_cases() {
        // ASSERT
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
    fn xxhash32_empty_input() {
        // ACT
        let hash = xxhash32(b"", 0);

        // ASSERT
        assert_ne!(hash, 0);
    }

    #[test]
    fn xxhash32_single_byte() {
        // ACT
        let hash = xxhash32(b"a", 0);

        // ASSERT
        assert_ne!(hash, 0);
    }

    #[test]
    fn xxhash32_four_bytes() {
        // ACT
        let hash = xxhash32(b"test", 0);

        // ASSERT
        assert_ne!(hash, 0);
    }

    #[test]
    fn xxhash32_eight_bytes() {
        // ACT
        let hash = xxhash32(b"12345678", 0);

        // ASSERT
        assert_ne!(hash, 0);
    }

    #[test]
    fn xxhash32_twelve_bytes() {
        // ACT
        let hash = xxhash32(b"123456789012", 0);

        // ASSERT
        assert_ne!(hash, 0);
    }

    #[test]
    fn xxhash32_sixteen_bytes() {
        // ACT
        let hash = xxhash32(b"1234567890123456", 0);

        // ASSERT
        assert_ne!(hash, 0);
    }

    #[test]
    fn xxhash32_seventeen_bytes() {
        // ACT
        let hash = xxhash32(b"12345678901234567", 0);

        // ASSERT
        assert_ne!(hash, 0);
    }

    #[test]
    fn xxhash32_thirty_two_bytes() {
        // ACT
        let hash = xxhash32(b"12345678901234567890123456789012", 0);

        // ASSERT
        assert_ne!(hash, 0);
    }

    #[test]
    fn xxhash32_different_seeds() {
        // ARRANGE
        let data = b"test data for hashing";

        // ACT
        let h1 = xxhash32(data, 0);
        let h2 = xxhash32(data, 100);
        let h3 = xxhash32(data, u32::MAX);

        // ASSERT
        assert_ne!(h1, h2);
        assert_ne!(h2, h3);
        assert_ne!(h1, h3);
    }

    #[test]
    fn xxhash32_consistent_for_same_input() {
        // ARRANGE
        let data = b"consistent input";

        // ACT
        let h1 = xxhash32(data, 42);
        let h2 = xxhash32(data, 42);

        // ASSERT
        assert_eq!(h1, h2);
    }

    #[test]
    fn build_selinux_xattr_empty_label() {
        // ACT
        let payload = build_selinux_xattr(b"");

        // ASSERT
        assert!(payload.len() >= XATTR_HEADER_SIZE + 4);
        assert_eq!(payload.len() % 4, 0);
    }

    #[test]
    fn build_selinux_xattr_one_byte_value() {
        // ACT
        let payload = build_selinux_xattr(b"x");

        // ASSERT
        assert!(payload.len() >= XATTR_HEADER_SIZE + 8);
        assert_eq!(payload.len() % 4, 0);
    }

    #[test]
    fn build_selinux_xattr_four_byte_value() {
        // ACT
        let payload = build_selinux_xattr(b"xxxx");

        // ASSERT
        assert!(payload.len() >= XATTR_HEADER_SIZE + 8);
        assert_eq!(payload.len() % 4, 0);
    }

    #[test]
    fn build_selinux_xattr_five_byte_value() {
        // ACT
        let payload = build_selinux_xattr(b"xxxxx");

        // ASSERT
        assert!(payload.len() >= XATTR_HEADER_SIZE + 12);
        assert_eq!(payload.len() % 4, 0);
    }

    #[test]
    fn xattr_icount_minimum_payload() {
        // ARRANGE
        let payload = build_selinux_xattr(b"x");

        // ACT
        let count = xattr_icount(payload.len());

        // ASSERT
        assert_eq!(count, 4);
    }

    #[test]
    fn xattr_icount_with_full_payload() {
        // ARRANGE
        let label = b"system_u:object_r:admin_home_t:s0";
        let payload = build_selinux_xattr(label);

        // ACT
        let count = xattr_icount(payload.len());

        // ASSERT
        assert_eq!(count, 12);
    }

    #[test]
    fn compute_name_filter_produces_valid_mask() {
        // ACT
        let filter = compute_name_filter(EROFS_XATTR_INDEX_SECURITY, b"selinux");

        // ASSERT
        assert_ne!(filter, u32::MAX);
        assert_ne!(filter, 0);
    }

    #[test]
    fn compute_name_filter_different_indices() {
        // ACT
        let f1 = compute_name_filter(EROFS_XATTR_INDEX_SECURITY, b"selinux");
        let f2 = compute_name_filter(EROFS_XATTR_INDEX_SECURITY + 1, b"selinux");

        // ASSERT
        assert_ne!(f1, f2);
    }
}
