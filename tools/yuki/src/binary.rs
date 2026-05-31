//! Utilities for binary data manipulation.

use crate::error::{Result, YukiError};

/// Aligns a value up to the nearest multiple of the given alignment.
#[inline]
pub(crate) const fn align_to(value: u32, alignment: u32) -> u32 {
    let alignment = if alignment == 0 { 1 } else { alignment };
    let mask = alignment.wrapping_sub(1);
    value.saturating_add(mask) & !mask
}

/// Reads a little-endian u32 from a byte buffer at the given offset.
#[inline]
pub(crate) fn read_u32(buf: &[u8], off: usize) -> Result<u32> {
    let end = off.saturating_add(4);
    let bytes = buf
        .get(off..end)
        .ok_or_else(|| YukiError::InvalidPeStructure(format!("u32 read oob: {off}-{end}")))?;
    let word_bytes: [u8; 4] = bytes
        .try_into()
        .map_err(|_error| YukiError::InvalidPeStructure("u32 read width mismatch".to_owned()))?;
    Ok(u32::from_le_bytes(word_bytes))
}

/// Writes a little-endian u16 to a byte buffer at the given offset.
#[inline]
pub(crate) fn write_u16(buf: &mut [u8], off: usize, val: u16) -> Result<()> {
    let end = off.saturating_add(2);
    let dst = buf
        .get_mut(off..end)
        .ok_or_else(|| YukiError::InvalidPeStructure(format!("u16 write oob: {off}-{end}")))?;
    dst.copy_from_slice(&val.to_le_bytes());
    Ok(())
}

/// Writes a little-endian u32 to a byte buffer at the given offset.
#[inline]
pub(crate) fn write_u32(buf: &mut [u8], off: usize, val: u32) -> Result<()> {
    let end = off.saturating_add(4);
    let dst = buf
        .get_mut(off..end)
        .ok_or_else(|| YukiError::InvalidPeStructure(format!("u32 write oob: {off}-{end}")))?;
    dst.copy_from_slice(&val.to_le_bytes());
    Ok(())
}

/// Converts a wide integer to `usize` with a contextual error.
#[inline]
pub(crate) fn usize_from_u128(value: u128, context: &'static str) -> Result<usize> {
    usize::try_from(value)
        .map_err(|_conversion_error| YukiError::InvalidPeStructure(context.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align_to_zero_alignment() {
        // ARRANGE
        let test_cases = vec![(100, 0, 100), (0, 0, 0), (u32::MAX, 0, u32::MAX)];

        // ACT
        for (value, alignment, expected) in test_cases {
            // ASSERT
            assert_eq!(align_to(value, alignment), expected);
        }
    }

    #[test]
    fn align_to_already_aligned() {
        // ARRANGE
        let test_cases = vec![(0, 4096, 0), (4096, 4096, 4096), (8192, 4096, 8192)];

        // ACT
        for (value, alignment, expected) in test_cases {
            // ASSERT
            assert_eq!(align_to(value, alignment), expected);
        }
    }

    #[test]
    fn align_to_needs_alignment() {
        // ARRANGE
        let test_cases = vec![(1, 4, 4), (100, 4, 100), (101, 4, 104), (4095, 4096, 4096)];

        // ACT
        for (value, alignment, expected) in test_cases {
            // ASSERT
            assert_eq!(align_to(value, alignment), expected);
        }
    }

    #[test]
    fn align_to_various_alignments() {
        // ARRANGE
        let test_cases = vec![
            (10, 8, 16),
            (16, 8, 16),
            (17, 8, 24),
            (50, 16, 64),
            (64, 16, 64),
        ];

        // ACT
        for (value, alignment, expected) in test_cases {
            // ASSERT
            assert_eq!(align_to(value, alignment), expected);
        }
    }

    #[test]
    fn align_to_power_of_two() {
        // ARRANGE
        let alignments = [2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096];

        // ACT
        for alignment in alignments {
            // ASSERT
            assert_eq!(align_to(0, alignment), 0);
            assert_eq!(align_to(alignment, alignment), alignment);
            assert_eq!(align_to(alignment * 2, alignment), alignment * 2);
            assert_eq!(align_to(1, alignment), alignment);
            assert_eq!(align_to(alignment - 1, alignment), alignment);
            assert_eq!(align_to(alignment + 1, alignment), alignment * 2);
        }
    }

    #[test]
    fn read_u32_basic() {
        // ARRANGE
        let buf = [0x78, 0x56, 0x34, 0x12, 0xFF, 0xFF];

        // ACT
        let value = read_u32(&buf, 0).unwrap_or_default();

        // ASSERT
        assert_eq!(value, 0x1234_5678);
    }

    #[test]
    fn read_u32_different_offsets() {
        // ARRANGE
        let buf = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        let test_cases = vec![(0, 0x3322_1100), (1, 0x4433_2211), (4, 0x7766_5544)];

        // ACT
        for (offset, expected) in test_cases {
            // ASSERT
            assert_eq!(read_u32(&buf, offset).unwrap_or_default(), expected);
        }
    }

    #[test]
    fn read_u32_all_zeros() {
        // ARRANGE
        let buf = [0x00, 0x00, 0x00, 0x00];

        // ACT
        let value = read_u32(&buf, 0).unwrap_or_default();

        // ASSERT
        assert_eq!(value, 0);
    }

    #[test]
    fn read_u32_all_ones() {
        // ARRANGE
        let buf = [0xFF, 0xFF, 0xFF, 0xFF];

        // ACT
        let value = read_u32(&buf, 0).unwrap_or_default();

        // ASSERT
        assert_eq!(value, 0xFFFF_FFFF);
    }

    #[test]
    fn read_u32_little_endian() {
        // ARRANGE
        let buf = [0x01, 0x02, 0x03, 0x04];

        // ACT
        let value = read_u32(&buf, 0).unwrap_or_default();

        // ASSERT
        assert_eq!(value, 0x0403_0201);
    }

    #[test]
    fn write_u32_basic() {
        // ARRANGE
        let mut buf = [0_u8; 4];

        // ACT
        write_u32(&mut buf, 0, 0x1234_5678).unwrap_or_default();

        // ASSERT
        assert_eq!(buf, [0x78, 0x56, 0x34, 0x12]);
    }

    #[test]
    fn write_u32_different_offsets() {
        // ARRANGE
        let mut buf = [0_u8; 8];

        // ACT
        write_u32(&mut buf, 0, 0x1122_3344).unwrap_or_default();
        write_u32(&mut buf, 4, 0x5566_7788).unwrap_or_default();

        // ASSERT
        assert_eq!(buf, [0x44, 0x33, 0x22, 0x11, 0x88, 0x77, 0x66, 0x55]);
    }

    #[test]
    fn write_u32_overwrites_correctly() {
        // ARRANGE
        let mut buf = [0xFF; 8];

        // ACT
        write_u32(&mut buf, 2, 0).unwrap_or_default();

        // ASSERT
        assert_eq!(buf[2..6], [0x00, 0x00, 0x00, 0x00]);
        assert_eq!(buf[0], 0xFF);
        assert_eq!(buf[1], 0xFF);
        assert_eq!(buf[6], 0xFF);
        assert_eq!(buf[7], 0xFF);
    }

    #[test]
    fn write_u32_zero_value() {
        // ARRANGE
        let mut buf = [0xFF; 4];

        // ACT
        write_u32(&mut buf, 0, 0).unwrap_or_default();

        // ASSERT
        assert_eq!(buf, [0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn write_u32_max_value() {
        // ARRANGE
        let mut buf = [0_u8; 4];

        // ACT
        write_u32(&mut buf, 0, 0xFFFF_FFFF).unwrap_or_default();

        // ASSERT
        assert_eq!(buf, [0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn read_write_u32_roundtrip() {
        // ARRANGE
        let mut buf = [0_u8; 4];
        let test_values = [
            0x0000_0000,
            0x1234_5678,
            0xDEAD_BEEF,
            0xFFFF_FFFF,
            0x0000_0001,
        ];

        // ACT
        for value in test_values {
            write_u32(&mut buf, 0, value).unwrap_or_default();

            // ASSERT
            assert_eq!(read_u32(&buf, 0).unwrap_or_default(), value);
        }
    }

    #[test]
    fn align_to_boundary_conditions() {
        // ARRANGE
        let test_cases = vec![
            (511, 512, 512),
            (512, 512, 512),
            (513, 512, 1024),
            (100, 1, 100),
            (0, 1, 0),
        ];

        // ACT
        for (value, alignment, expected) in test_cases {
            // ASSERT
            assert_eq!(align_to(value, alignment), expected);
        }
    }

    #[test]
    fn write_u16_basic() {
        // ARRANGE
        let mut buf = [0_u8; 2];

        // ACT
        write_u16(&mut buf, 0, 0x1234).unwrap_or_default();

        // ASSERT
        assert_eq!(buf, [0x34, 0x12]);
    }
    #[test]
    fn write_u16_rejects_out_of_bounds() {
        // ARRANGE
        let mut buf = [0_u8; 1];

        // ACT
        let result = write_u16(&mut buf, 0, 0x1234);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn read_u32_rejects_out_of_bounds() {
        // ARRANGE
        let buf = [0_u8; 3];

        // ACT
        let result = read_u32(&buf, 0);

        // ASSERT
        assert!(result.is_err(), "out of bounds read should fail");
    }

    #[test]
    fn write_u32_rejects_out_of_bounds() {
        // ARRANGE
        let mut buf = [0_u8; 3];

        // ACT
        let result = write_u32(&mut buf, 0, 1);

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn usize_from_u128_accepts_small_value() {
        // ARRANGE
        let value = 123_u128;

        // ACT
        let result = usize_from_u128(value, "conversion failed").unwrap_or_default();

        // ASSERT
        assert_eq!(result, 123);
    }

    #[test]
    fn usize_from_u128_rejects_large_value() {
        // ARRANGE
        let value = u128::MAX;

        // ACT
        let result = usize_from_u128(value, "conversion failed");

        // ASSERT
        assert!(matches!(
            result,
            Err(YukiError::InvalidPeStructure(message)) if message == "conversion failed"
        ));
    }
}
