//! Shared utility helpers

/// Strips trailing NUL and ASCII whitespace bytes from a command line.
pub fn strip_trailing_cmdline_terminators(data: &[u8]) -> &[u8] {
    let end = data
        .iter()
        .rposition(|byte| *byte != 0 && !byte.is_ascii_whitespace())
        .map_or(0, |index| index + 1);
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
        assert_eq!(strip_trailing_cmdline_terminators(input), b"");
    }

    #[test]
    fn all_terminators_returns_empty() {
        // ARRANGE
        let input = b" \n\t\0\0";
        // ACT + ASSERT
        assert_eq!(strip_trailing_cmdline_terminators(input), b"");
    }

    #[test]
    fn no_terminators_returned_unchanged() {
        // ARRANGE
        let input = b"hello";
        // ACT + ASSERT
        assert_eq!(strip_trailing_cmdline_terminators(input), b"hello");
    }

    #[test]
    fn trailing_nuls_stripped() {
        // ARRANGE
        let input = b"hello\0\0";
        // ACT + Assert
        assert_eq!(strip_trailing_cmdline_terminators(input), b"hello");
    }

    #[test]
    fn trailing_newline_stripped() {
        // ARRANGE
        let input = b"console=ttyS0\n";
        // ACT + Assert
        assert_eq!(strip_trailing_cmdline_terminators(input), b"console=ttyS0");
    }

    #[test]
    fn single_nul_stripped() {
        // ARRANGE
        let input = b"x\0";
        // ACT + ASSERT
        assert_eq!(strip_trailing_cmdline_terminators(input), b"x");
    }

    #[test]
    fn nul_in_middle_not_stripped() {
        // ARRANGE
        let input = b"hel\0lo";
        // ACT + ASSERT
        assert_eq!(strip_trailing_cmdline_terminators(input), b"hel\0lo");
    }

    #[test]
    fn nul_only_in_middle_not_stripped() {
        // ARRANGE
        let input = b"a\0b";
        // ACT + ASSERT
        assert_eq!(strip_trailing_cmdline_terminators(input), b"a\0b");
    }
}
