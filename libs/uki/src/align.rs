use crate::error::{Result, UkiError};

/// Rounds `value` up to the nearest multiple of `alignment`.
///
/// `alignment` must be a power of two, or 0 (which is treated as 1).
#[must_use]
pub const fn to(value: u32, alignment: u32) -> u32 {
    let alignment = if alignment == 0 { 1 } else { alignment };
    let mask = alignment.wrapping_sub(1);

    value.saturating_add(mask) & !mask
}

/// Rounds `value` up to the nearest multiple of `alignment`.
///
/// # Errors
///
/// Returns `Err` if `alignment` is zero or if the result overflows `usize`.
pub fn up(value: usize, alignment: usize) -> Result<usize> {
    let Some(alignment_mask) = alignment.checked_sub(1) else {
        return Err(UkiError::InvalidPe("zero alignment"));
    };
    let adjusted = value
        .checked_add(alignment_mask)
        .ok_or(UkiError::Overflow("alignment"))?;

    Ok(adjusted & !alignment_mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align_to_zero_alignment_returns_value() {
        // ARRANGE
        let test_cases = [
            (100_u32, 0_u32, 100_u32),
            (0, 0, 0),
            (u32::MAX, 0, u32::MAX),
        ];

        // ACT & ASSERT
        for (value, alignment, expected) in test_cases {
            assert_eq!(to(value, alignment), expected);
        }
    }

    #[test]
    fn align_to_rounds_up() {
        // ARRANGE
        let test_cases = [
            (1_u32, 4_u32, 4_u32),
            (100, 4, 100),
            (101, 4, 104),
            (4095, 4096, 4096),
            (512, 512, 512),
            (513, 512, 1024),
        ];

        // ACT & ASSERT
        for (value, alignment, expected) in test_cases {
            assert_eq!(to(value, alignment), expected);
        }
    }

    #[test]
    fn align_up_rejects_zero_alignment() {
        // ARRANGE
        let value = 5_usize;
        let alignment = 0_usize;

        // ACT
        let result = up(value, alignment);

        // ASSERT
        assert!(matches!(result, Err(UkiError::InvalidPe(_))));
    }

    #[test]
    fn align_up_rounds_up() {
        // ARRANGE
        let test_cases = [(9_usize, 8_usize, 16_usize), (0, 8, 0), (16, 8, 16)];

        // ACT & ASSERT
        for (value, alignment, expected) in test_cases {
            assert_eq!(up(value, alignment).unwrap(), expected);
        }
    }

    #[test]
    fn align_up_rejects_overflow() {
        // ARRANGE
        let value = usize::MAX;
        let alignment = 8_usize;

        // ACT
        let result = up(value, alignment);

        // ASSERT
        assert!(matches!(result, Err(UkiError::Overflow(_))));
    }
}
