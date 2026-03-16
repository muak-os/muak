//! Shared utility helpers

/// Strips trailing NUL bytes from a byte slice.
pub fn strip_trailing_nuls(data: &[u8]) -> &[u8] {
    let end = data.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    &data[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_slice_returns_empty() {
        // ARRANGE
        let input = b"";
        // ACT + ASSERT
        assert_eq!(strip_trailing_nuls(input), b"");
    }

    #[test]
    fn all_nuls_returns_empty() {
        // ARRANGE
        let input = b"\0\0\0";
        // ACT + ASSERT
        assert_eq!(strip_trailing_nuls(input), b"");
    }

    #[test]
    fn no_nuls_returned_unchanged() {
        // ARRANGE
        let input = b"hello";
        // ACT + ASSERT
        assert_eq!(strip_trailing_nuls(input), b"hello");
    }

    #[test]
    fn trailing_nuls_stripped() {
        // ARRANGE
        let input = b"hello\0\0";
        // ACT + Assert
        assert_eq!(strip_trailing_nuls(input), b"hello");
    }

    #[test]
    fn single_nul_stripped() {
        // ARRANGE
        let input = b"x\0";
        // ACT + ASSERT
        assert_eq!(strip_trailing_nuls(input), b"x");
    }

    #[test]
    fn nul_in_middle_not_stripped() {
        // ARRANGE
        let input = b"hel\0lo";
        // ACT + ASSERT
        assert_eq!(strip_trailing_nuls(input), b"hel\0lo");
    }

    #[test]
    fn nul_only_in_middle_not_stripped() {
        // ARRANGE
        let input = b"a\0b";
        // ACT + ASSERT
        assert_eq!(strip_trailing_nuls(input), b"a\0b");
    }
}
