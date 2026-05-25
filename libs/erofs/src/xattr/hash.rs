//! Minimal xxHash32 implementation used by xattr name filters.

use crate::checked::{read_bytes, u32_from_usize};

/// Minimal xxHash32 implementation for xattr name filter.
pub(super) fn xxhash32(input: &[u8], seed: u32) -> u32 {
    const PRIME1: u32 = 0x9E37_79B1;
    const PRIME2: u32 = 0x85EB_CA77;
    const PRIME3: u32 = 0xC2B2_AE3D;
    const PRIME4: u32 = 0x27D4_EB2F;
    const PRIME5: u32 = 0x1656_67B1;

    let len = u32_from_usize(input.len()).unwrap_or(u32::MAX);
    let mut hash_accumulator: u32;
    let mut offset = 0_usize;

    if input.len() >= 16 {
        let mut v1 = seed.wrapping_add(PRIME1).wrapping_add(PRIME2);
        let mut v2 = seed.wrapping_add(PRIME2);
        let mut v3 = seed;
        let mut v4 = seed.wrapping_sub(PRIME1);

        while offset.saturating_add(16) <= input.len() {
            v1 = xxh32_round(v1, read_u32_le(input, offset));
            v2 = xxh32_round(v2, read_u32_le(input, offset.saturating_add(4)));
            v3 = xxh32_round(v3, read_u32_le(input, offset.saturating_add(8)));
            v4 = xxh32_round(v4, read_u32_le(input, offset.saturating_add(12)));
            offset = offset.saturating_add(16);
        }
        hash_accumulator = v1
            .rotate_left(1)
            .wrapping_add(v2.rotate_left(7))
            .wrapping_add(v3.rotate_left(12))
            .wrapping_add(v4.rotate_left(18));
    } else {
        hash_accumulator = seed.wrapping_add(PRIME5);
    }

    hash_accumulator = hash_accumulator.wrapping_add(len);

    while offset.saturating_add(4) <= input.len() {
        hash_accumulator =
            hash_accumulator.wrapping_add(read_u32_le(input, offset).wrapping_mul(PRIME3));
        hash_accumulator = hash_accumulator.rotate_left(17).wrapping_mul(PRIME4);
        offset = offset.saturating_add(4);
    }

    while let Some(&byte) = input.get(offset) {
        hash_accumulator = hash_accumulator.wrapping_add(u32::from(byte).wrapping_mul(PRIME5));
        hash_accumulator = hash_accumulator.rotate_left(11).wrapping_mul(PRIME1);
        offset = offset.saturating_add(1);
    }

    hash_accumulator ^= hash_accumulator >> 15;
    hash_accumulator = hash_accumulator.wrapping_mul(PRIME2);
    hash_accumulator ^= hash_accumulator >> 13;
    hash_accumulator = hash_accumulator.wrapping_mul(PRIME3);
    hash_accumulator ^= hash_accumulator >> 16;
    hash_accumulator
}

fn xxh32_round(acc: u32, input: u32) -> u32 {
    acc.wrapping_add(input.wrapping_mul(0x85EB_CA77))
        .rotate_left(13)
        .wrapping_mul(0x9E37_79B1)
}

pub(super) fn read_u32_le(buf: &[u8], offset: usize) -> u32 {
    read_bytes::<4>(buf, offset).map_or(0, u32::from_le_bytes)
}

#[cfg(test)]
mod tests {
    use super::{read_u32_le, xxhash32};

    #[test]
    fn xxhash32_empty_input() {
        // ARRANGE
        // ACT
        // ASSERT
        assert_ne!(xxhash32(b"", 0), 0);
    }

    #[test]
    fn xxhash32_single_byte() {
        // ARRANGE
        // ACT
        // ASSERT
        assert_ne!(xxhash32(b"a", 0), 0);
    }

    #[test]
    fn xxhash32_four_bytes() {
        // ARRANGE
        // ACT
        // ASSERT
        assert_ne!(xxhash32(b"test", 0), 0);
    }

    #[test]
    fn xxhash32_eight_bytes() {
        // ARRANGE
        // ACT
        // ASSERT
        assert_ne!(xxhash32(b"12345678", 0), 0);
    }

    #[test]
    fn xxhash32_twelve_bytes() {
        // ARRANGE
        // ACT
        // ASSERT
        assert_ne!(xxhash32(b"123456789012", 0), 0);
    }

    #[test]
    fn xxhash32_sixteen_bytes() {
        // ARRANGE
        // ACT
        // ASSERT
        assert_ne!(xxhash32(b"1234567890123456", 0), 0);
    }

    #[test]
    fn xxhash32_seventeen_bytes() {
        // ARRANGE
        // ACT
        // ASSERT
        assert_ne!(xxhash32(b"12345678901234567", 0), 0);
    }

    #[test]
    fn xxhash32_thirty_two_bytes() {
        // ARRANGE
        // ACT
        // ASSERT
        assert_ne!(xxhash32(b"12345678901234567890123456789012", 0), 0);
    }

    #[test]
    fn xxhash32_different_seeds() {
        // ARRANGE
        let data = b"test data for hashing";
        let first = xxhash32(data, 0);
        let second = xxhash32(data, 100);
        let third = xxhash32(data, u32::MAX);
        // ACT
        // ASSERT
        assert_ne!(first, second);
        assert_ne!(second, third);
        assert_ne!(first, third);
    }

    #[test]
    fn xxhash32_consistent_for_same_input() {
        // ARRANGE
        let data = b"consistent input";
        let first = xxhash32(data, 42);
        let second = xxhash32(data, 42);
        // ACT
        // ASSERT
        assert_eq!(first, second);
    }

    #[test]
    fn read_u32_le_returns_zero_when_offset_is_out_of_bounds() {
        // ARRANGE
        let buf = [1_u8, 2, 3];
        let value = read_u32_le(&buf, 1);
        // ACT
        // ASSERT
        assert_eq!(value, 0);
    }
}
