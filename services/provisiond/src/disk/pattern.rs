//! Minimal glob matching for disk matchers, supporting `*` and `?` only.

/// Returns true when `value` matches `pattern`, where `*` matches any run of
/// characters and `?` matches exactly one character.
pub(crate) fn matches(pattern: &str, value: &str) -> bool {
    let Some(token) = pattern.chars().next() else {
        return value.is_empty();
    };
    let rest = pattern.get(token.len_utf8()..).unwrap_or("");

    match token {
        '*' => {
            matches(rest, value)
                || value.chars().next().is_some_and(|first| {
                    matches(pattern, value.get(first.len_utf8()..).unwrap_or(""))
                })
        }
        '?' => value
            .chars()
            .next()
            .is_some_and(|first| matches(rest, value.get(first.len_utf8()..).unwrap_or(""))),
        literal => value
            .strip_prefix(literal)
            .is_some_and(|tail| matches(rest, tail)),
    }
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn literal_pattern_matches_exact_value() {
        // ARRANGE
        let pattern = "S3Z8NX0M";

        // ACT & ASSERT
        assert!(matches(pattern, "S3Z8NX0M"));
        assert!(!matches(pattern, "S3Z8NX0N"));
        assert!(!matches(pattern, "S3Z8NX0MX"));
    }

    #[test]
    fn star_matches_any_run_including_empty() {
        // ARRANGE
        let pattern = "nvme*";

        // ACT & ASSERT
        assert!(matches(pattern, "nvme0n1"));
        assert!(matches(pattern, "nvme"));
        assert!(!matches(pattern, "sda"));
    }

    #[test]
    fn question_matches_exactly_one_character() {
        // ARRANGE
        let pattern = "sd?";

        // ACT & ASSERT
        assert!(matches(pattern, "sda"));
        assert!(!matches(pattern, "sd"));
        assert!(!matches(pattern, "sdab"));
    }

    #[test]
    fn mixed_wildcards_match_expected_values() {
        // ARRANGE
        let pattern = "naa.5000*?A";

        // ACT & ASSERT
        assert!(matches(pattern, "naa.5000abcA"));
        assert!(!matches(pattern, "naa.5000abc"));
        assert!(!matches(pattern, "naa.6000abcA"));
    }

    #[test]
    fn empty_pattern_matches_only_empty_value() {
        // ACT & ASSERT
        assert!(matches("", ""));
        assert!(!matches("", "sda"));
    }
}
