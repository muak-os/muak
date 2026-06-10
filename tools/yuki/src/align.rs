//! Alignment and numeric conversion utilities for PE data layout.

use crate::error::{Result, YukiError};

/// Rounds `value` up to the nearest multiple of `alignment`.
///
/// `alignment` must be a power of two, or 0 (which is treated as 1).
#[inline]
pub const fn align_to(value: u32, alignment: u32) -> u32 {
    let alignment = if alignment == 0 { 1 } else { alignment };
    let mask = alignment.wrapping_sub(1);

    value.saturating_add(mask) & !mask
}

pub fn usize_to_u32(value: usize) -> Result<u32> {
    u32::try_from(value).map_err(|_err| YukiError::PeParseError("usize to u32 overflow".to_owned()))
}

pub fn u64_to_usize(value: u64) -> Result<usize> {
    usize::try_from(value)
        .map_err(|_err| YukiError::PeParseError("u64 to usize overflow".to_owned()))
}

pub fn validate_section_size(len: u64, name: &'static str) -> Result<usize> {
    let len = usize::try_from(len).map_err(|_err| {
        YukiError::InvalidPeStructure(format!("section '{name}' length exceeds usize"))
    })?;
    u32::try_from(len)
        .map_err(|_err| YukiError::InvalidPeStructure(format!("section '{name}' too large")))?;

    Ok(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align_to_zero_alignment_returns_value_unchanged() {
        // ARRANGE
        let test_cases = vec![(100, 0, 100), (0, 0, 0), (u32::MAX, 0, u32::MAX)];

        // ACT
        for (value, alignment, expected) in test_cases {
            // ASSERT
            assert_eq!(align_to(value, alignment), expected);
        }
    }

    #[test]
    fn align_to_rounds_up_to_alignment_boundary() {
        // ARRANGE
        let test_cases = vec![
            (1, 4, 4),
            (100, 4, 100),
            (101, 4, 104),
            (4095, 4096, 4096),
            (511, 512, 512),
            (512, 512, 512),
            (513, 512, 1024),
            (100, 1, 100),
        ];

        // ACT
        for (value, alignment, expected) in test_cases {
            // ASSERT
            assert_eq!(align_to(value, alignment), expected);
        }
    }
}
