//! Utilities for binary data manipulation.

use crate::error::{Result, YukiError};

/// Aligns a value up to the nearest multiple of the given alignment.
#[inline]
pub(crate) const fn align_to(value: u32, alignment: u32) -> u32 {
    let alignment = if alignment == 0 { 1 } else { alignment };
    let mask = alignment.wrapping_sub(1);

    value.saturating_add(mask) & !mask
}

/// Converts a `u128` value to usize, returning an error if the value exceeds usize::MAX.
#[inline]
pub(crate) fn usize_from_u128(value: u128, context: &'static str) -> Result<usize> {
    usize::try_from(value)
        .map_err(|_conversion_error| YukiError::InvalidPeStructure(context.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usize_from_u128_accepts_small_value() {
        // ARRANGE
        let value = 123_u128;

        // ACT
        let result =
            usize_from_u128(value, "conversion failed").expect("small value should convert");

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
}
