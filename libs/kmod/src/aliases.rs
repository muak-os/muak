//! Module alias database support.

use std::fs;
use std::path::Path;

use crate::text::{self, Span};

const ALIAS_PREFIX: &str = "alias ";

/// Parsed `modules.alias` database.
#[derive(Debug)]
pub struct AliasDb {
    text: String,
    entries: Vec<Alias>,
}

#[derive(Debug)]
struct Alias {
    pattern: Span,
    module: Span,
}

impl AliasDb {
    /// Loads a `modules.alias` database from disk.
    ///
    /// # Errors
    ///
    /// Returns an error when `path` cannot be opened or read.
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let text = fs::read_to_string(path)?;

        Ok(Self {
            entries: parse_text(&text),
            text,
        })
    }

    /// Finds the first module whose alias pattern matches `modalias`.
    #[must_use]
    pub fn find_module(&self, modalias: &str) -> Option<&str> {
        self.entries.iter().find_map(|alias| {
            glob_match_bytes(
                text::slice(&self.text, alias.pattern).as_bytes(),
                modalias.as_bytes(),
            )
            .then_some(text::slice(&self.text, alias.module))
        })
    }

    /// Returns the number of alias entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the alias database is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn parse_text(text: &str) -> Vec<Alias> {
    let mut entries = Vec::new();
    text::for_each_line(text, |line, offset| {
        if let Some(alias) = parse_alias(line, offset) {
            entries.push(alias);
        }
    });

    entries
}

fn parse_alias(line: &str, offset: usize) -> Option<Alias> {
    let lead = line.len().saturating_sub(line.trim_start().len());
    let line = line.trim();
    let rest = line.strip_prefix(ALIAS_PREFIX)?;
    let (pattern, module) = rest.rsplit_once(' ')?;

    let pattern_base = offset
        .saturating_add(lead)
        .saturating_add(ALIAS_PREFIX.len());

    Some(Alias {
        pattern: text::span_at(pattern_base, pattern),
        module: text::span_at(
            pattern_base.saturating_add(pattern.len()).saturating_add(1),
            module,
        ),
    })
}

fn glob_match_bytes(pattern: &[u8], text: &[u8]) -> bool {
    let mut pattern_index = 0;
    let mut text_index = 0;
    let mut star_pattern_index = None;
    let mut star_text_index = None;

    while let Some(&text_byte) = text.get(text_index) {
        let matched = match pattern.get(pattern_index).copied() {
            Some(b'*') => {
                star_pattern_index = Some(pattern_index);
                star_text_index = Some(text_index);
                pattern_index = pattern_index.saturating_add(1);
                true
            }
            Some(b'?') => {
                pattern_index = pattern_index.saturating_add(1);
                text_index = text_index.saturating_add(1);
                true
            }
            Some(pattern_byte)
                if pattern_byte.to_ascii_lowercase() == text_byte.to_ascii_lowercase() =>
            {
                pattern_index = pattern_index.saturating_add(1);
                text_index = text_index.saturating_add(1);
                true
            }
            _ => false,
        };

        if matched {
            continue;
        }

        let Some((saved_pattern_index, saved_text_index)) = star_pattern_index.zip(star_text_index)
        else {
            return false;
        };
        pattern_index = saved_pattern_index.saturating_add(1);
        text_index = saved_text_index.saturating_add(1);
        star_text_index = Some(text_index);
    }

    while pattern
        .get(pattern_index)
        .is_some_and(|pattern_byte| *pattern_byte == b'*')
    {
        pattern_index = pattern_index.saturating_add(1);
    }

    pattern_index == pattern.len()
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use tempfile::NamedTempFile;

    use super::*;

    fn glob_match(pattern: &str, text: &str) -> bool {
        glob_match_bytes(pattern.as_bytes(), text.as_bytes())
    }

    #[test]
    fn glob_exact() {
        // ARRANGE
        let test_cases = [
            ("foo", "foo", true),
            ("foo", "FOO", true),
            ("foo", "bar", false),
            ("foo", "foobar", false),
        ];

        // ACT & ASSERT
        for (pattern, text, expected) in test_cases {
            assert_eq!(glob_match(pattern, text), expected, "{pattern} vs {text}");
        }
    }

    #[test]
    fn glob_star() {
        // ARRANGE
        let test_cases = [
            ("*", "anything", true),
            ("foo*", "foobar", true),
            ("*bar", "foobar", true),
            ("foo*bar", "fooXXXbar", true),
            ("foo*bar", "foobar", true),
            ("foo*bar", "foobaz", false),
        ];

        // ACT & ASSERT
        for (pattern, text, expected) in test_cases {
            assert_eq!(glob_match(pattern, text), expected, "{pattern} vs {text}");
        }
    }

    #[test]
    fn glob_question() {
        // ARRANGE
        let test_cases = [
            ("fo?", "foo", true),
            ("f??", "foo", true),
            ("fo?", "fo", false),
            ("fo?", "fooo", false),
            ("f??", "FOO", true),
        ];

        // ACT & ASSERT
        for (pattern, text, expected) in test_cases {
            assert_eq!(glob_match(pattern, text), expected, "{pattern} vs {text}");
        }
    }

    #[test]
    fn glob_empty_strings() {
        // ARRANGE
        let test_cases = [
            ("", "", true),
            ("", "foo", false),
            ("foo", "", false),
            ("*", "", true),
        ];

        // ACT & ASSERT
        for (pattern, text, expected) in test_cases {
            assert_eq!(glob_match(pattern, text), expected, "{pattern} vs {text}");
        }
    }

    #[test]
    fn glob_multiple_stars() {
        // ARRANGE
        let test_cases = [
            ("*foo*bar*", "XXXfooYYYbarZZZ", true),
            ("*foo*bar*", "foobar", true),
            ("**", "anything", true),
            ("a*b*c", "abc", true),
            ("a*b*c", "aXXXbYYYc", true),
        ];

        // ACT & ASSERT
        for (pattern, text, expected) in test_cases {
            assert_eq!(glob_match(pattern, text), expected, "{pattern} vs {text}");
        }
    }

    #[test]
    fn glob_star_and_question_combined() {
        // ARRANGE
        let test_cases = [
            ("a?c*", "abcdef", true),
            ("*?c", "abc", true),
            ("a*?", "ab", true),
            ("a*?", "abcd", true),
            ("a*?", "a", false),
        ];

        // ACT & ASSERT
        for (pattern, text, expected) in test_cases {
            assert_eq!(glob_match(pattern, text), expected, "{pattern} vs {text}");
        }
    }

    #[test]
    fn glob_trailing_star() {
        // ARRANGE
        let test_cases = [("foo*", "foo", true), ("foo*", "foobar", true)];

        // ACT & ASSERT
        for (pattern, text, expected) in test_cases {
            assert_eq!(glob_match(pattern, text), expected, "{pattern} vs {text}");
        }
    }

    #[test]
    fn glob_leading_star() {
        // ARRANGE
        let test_cases = [
            ("*foo", "foo", true),
            ("*foo", "barfoo", true),
            ("*foo", "foobar", false),
        ];

        // ACT & ASSERT
        for (pattern, text, expected) in test_cases {
            assert_eq!(glob_match(pattern, text), expected, "{pattern} vs {text}");
        }
    }

    #[test]
    fn glob_backtracking() {
        // ARRANGE
        let test_cases = [
            ("a*a", "aa", true),
            ("a*a", "aXa", true),
            ("a*a", "aXXXa", true),
            ("*a*a*a*", "aaa", true),
            ("*a*a*a*", "XaYaZaW", true),
        ];

        // ACT & ASSERT
        for (pattern, text, expected) in test_cases {
            assert_eq!(glob_match(pattern, text), expected, "{pattern} vs {text}");
        }
    }

    #[test]
    fn pci_modalias() {
        // ARRANGE
        let pattern = "pci:v00008086d00001521sv*sd*bc*sc*i*";
        let test_cases = [
            (
                "pci:v00008086d00001521sv00001028sd00000001bc02sc00i00",
                true,
            ),
            (
                "pci:v00008086d00001522sv00001028sd00000001bc02sc00i00",
                false,
            ),
        ];

        // ACT & ASSERT
        for (modalias, expected) in test_cases {
            assert_eq!(glob_match(pattern, modalias), expected);
        }
    }

    #[test]
    fn intel_i226v_modalias_is_case_insensitive() {
        // ARRANGE
        let pattern = "pci:v00008086d0000125Csv*sd*bc*sc*i*";
        let modalias = "pci:v00008086d0000125Csv00001043sd000087D2bc02sc00i00";
        let modalias_lower = "pci:v00008086d0000125csv00001043sd000087d2bc02sc00i00";

        // ACT & ASSERT
        assert!(glob_match(pattern, modalias));
        assert!(glob_match(pattern, modalias_lower));
    }

    #[test]
    fn usb_modalias() {
        // ARRANGE
        let pattern = "usb:v*p*d*dc*dsc*dp*ic03isc01ip01*";
        let modalias = "usb:v046DpC52Bd2111dc00dsc00dp00ic03isc01ip01in00";

        // ACT / ASSERT
        assert!(glob_match(pattern, modalias));
    }

    #[test]
    fn acpi_modalias() {
        // ARRANGE
        let pattern = "acpi:ACPI0003:";
        let test_cases = [("acpi:ACPI0003:", true), ("acpi:ACPI0004:", false)];

        // ACT & ASSERT
        for (modalias, expected) in test_cases {
            assert_eq!(glob_match(pattern, modalias), expected);
        }
    }

    #[test]
    fn parse_alias_spans_reference_trimmed_fields() {
        // ARRANGE
        let text = "alias pci:v00008086d* igb\n# comment\n\n  alias   usb:v*p*   usbhid  ";

        // ACT
        let db_entries = parse_text(text);

        // ASSERT
        assert_eq!(db_entries.len(), 2);
        let first = db_entries.first().expect("first alias");
        let second = db_entries.get(1).expect("second alias");
        assert_eq!(text::slice(text, first.pattern), "pci:v00008086d*");
        assert_eq!(text::slice(text, first.module), "igb");
        assert_eq!(text::slice(text, second.pattern), "usb:v*p*");
        assert_eq!(text::slice(text, second.module), "usbhid");
    }

    #[test]
    fn parse_alias_rejects_non_alias_lines() {
        // ARRANGE
        let text = "\n# comment\nnot an alias\nalias pattern_without_module\n";

        // ACT
        let db_entries = parse_text(text);

        // ASSERT
        assert!(db_entries.is_empty());
    }

    fn load_from(contents: &str) -> AliasDb {
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        writeln!(file, "{contents}").expect("write failed");
        AliasDb::load(file.path()).expect("load failed")
    }

    #[test]
    fn alias_db_case_insensitive() {
        // ARRANGE
        let db = load_from("alias pci:v00008086d0000125Csv*sd*bc*sc*i* igc");
        let modalias = "pci:v00008086d0000125csv00001043sd000087d2bc02sc00i00";

        // ACT
        let result = db.find_module(modalias);

        // ASSERT
        assert_eq!(result, Some("igc"));
    }

    #[test]
    fn alias_db_empty_file() {
        // ARRANGE
        let file = NamedTempFile::new().expect("Failed to create temp file");

        // ACT
        let db = AliasDb::load(file.path()).expect("load failed");

        // ASSERT
        assert!(db.is_empty());
        assert_eq!(db.len(), 0);
        assert_eq!(db.find_module("anything"), None);
    }

    #[test]
    fn alias_db_with_comments_and_blanks() {
        // ARRANGE
        let db = load_from(
            "# Comment line\n\nalias pattern1 module1\n  # Another comment\nalias pattern2 module2\n",
        );

        // ACT / ASSERT
        assert_eq!(db.len(), 2);
        assert_eq!(db.find_module("pattern1"), Some("module1"));
        assert_eq!(db.find_module("pattern2"), Some("module2"));
    }

    #[test]
    fn alias_db_multiple_entries() {
        // ARRANGE
        let db = load_from(
            "alias pci:v00008086d00001521* igb\nalias pci:v00008086d0000125C* igc\nalias pci:v000010DE* nvidia\n",
        );

        // ACT / ASSERT
        assert_eq!(db.len(), 3);
        assert!(!db.is_empty());

        assert_eq!(db.find_module("pci:v00008086d00001521sv1234"), Some("igb"));
        assert_eq!(db.find_module("pci:v00008086d0000125csv1234"), Some("igc"));
        assert_eq!(db.find_module("pci:v000010deABCD"), Some("nvidia"));
        assert_eq!(db.find_module("pci:v00001234d5678"), None);
    }

    #[test]
    fn alias_db_first_match_wins() {
        // ARRANGE
        let db = load_from("alias pci:* first_module\nalias pci:v00008086* second_module");

        // ACT
        let result = db.find_module("pci:v00008086d1234");

        // ASSERT
        assert_eq!(result, Some("first_module"));
    }

    #[test]
    fn alias_db_load_nonexistent_file() {
        // ACT / ASSERT
        let result = AliasDb::load(Path::new("/nonexistent/path/modules.alias"));
        result.expect_err("load should fail for nonexistent file");
    }

    #[test]
    fn alias_db_real_world_patterns() {
        // ARRANGE
        let db = load_from(
            "alias usb:v*p*d*dc*dsc*dp*ic03isc01ip01* usbhid\nalias acpi*:ACPI0003:* ac\nalias platform:efi-framebuffer efifb\n",
        );

        // ACT / ASSERT
        assert_eq!(
            db.find_module("usb:v046dpC52bd2111dc00dsc00dp00ic03isc01ip01in00"),
            Some("usbhid")
        );
        assert_eq!(db.find_module("acpi:acpi0003:"), Some("ac"));
        assert_eq!(db.find_module("platform:efi-framebuffer"), Some("efifb"));
    }
}
