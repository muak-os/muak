use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Debug)]
pub struct AliasDb {
    entries: Vec<(String, String)>, // (pattern, module)
}

fn parse_alias_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let rest = line.strip_prefix("alias ")?;
    let (pattern, module) = rest.rsplit_once(' ')?;
    Some((pattern.trim().to_string(), module.trim().to_string()))
}

impl AliasDb {
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let entries: Vec<_> = reader
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| parse_alias_line(&line))
            .collect();

        Ok(Self { entries })
    }

    pub fn find_module(&self, modalias: &str) -> Option<&str> {
        let modalias_lower = modalias.to_ascii_lowercase();
        self.entries
            .iter()
            .find(|(pattern, _)| {
                let pattern_lower = pattern.to_ascii_lowercase();
                glob_match_bytes(pattern_lower.as_bytes(), modalias_lower.as_bytes())
            })
            .map(|(_, module)| module.as_str())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn glob_match_bytes(pattern: &[u8], text: &[u8]) -> bool {
    let mut p = 0;
    let mut t = 0;
    let mut star_p = None;
    let mut star_t = None;

    while t < text.len() {
        let matched = match pattern.get(p) {
            Some(b'*') => {
                star_p = Some(p);
                star_t = Some(t);
                p += 1;
                true
            }
            Some(b'?') => {
                p += 1;
                t += 1;
                true
            }
            Some(&c) if c == text[t] => {
                p += 1;
                t += 1;
                true
            }
            _ => false,
        };

        if matched {
            continue;
        }

        let Some((sp, st)) = star_p.zip(star_t) else {
            return false;
        };
        p = sp + 1;
        star_t = Some(st + 1);
        t = st + 1;
    }

    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }

    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;

    fn glob_match(pattern: &str, text: &str) -> bool {
        glob_match_bytes(pattern.as_bytes(), text.as_bytes())
    }

    fn glob_match_icase(pattern: &str, text: &str) -> bool {
        let pattern_lower = pattern.to_ascii_lowercase();
        let text_lower = text.to_ascii_lowercase();
        glob_match_bytes(pattern_lower.as_bytes(), text_lower.as_bytes())
    }

    #[test]
    fn test_glob_exact() {
        // ARRANGE
        let test_cases = vec![
            ("foo", "foo", true),
            ("foo", "bar", false),
            ("foo", "foobar", false),
        ];

        // ACT & ASSERT
        for (pattern, text, expected) in test_cases {
            assert_eq!(glob_match(pattern, text), expected);
        }
    }

    #[test]
    fn test_glob_star() {
        // ARRANGE
        let test_cases = vec![
            ("*", "anything", true),
            ("foo*", "foobar", true),
            ("*bar", "foobar", true),
            ("foo*bar", "fooXXXbar", true),
            ("foo*bar", "foobar", true),
            ("foo*bar", "foobaz", false),
        ];

        // ACT & ASSERT
        for (pattern, text, expected) in test_cases {
            assert_eq!(glob_match(pattern, text), expected);
        }
    }

    #[test]
    fn test_glob_question() {
        // ARRANGE
        let test_cases = vec![
            ("fo?", "foo", true),
            ("f??", "foo", true),
            ("fo?", "fo", false),
            ("fo?", "fooo", false),
        ];

        // ACT & ASSERT
        for (pattern, text, expected) in test_cases {
            assert_eq!(glob_match(pattern, text), expected);
        }
    }

    #[test]
    fn test_glob_empty_strings() {
        // ARRANGE
        let test_cases = vec![
            ("", "", true),
            ("", "foo", false),
            ("foo", "", false),
            ("*", "", true),
        ];

        // ACT & ASSERT
        for (pattern, text, expected) in test_cases {
            assert_eq!(glob_match(pattern, text), expected);
        }
    }

    #[test]
    fn test_glob_multiple_stars() {
        // ARRANGE
        let test_cases = vec![
            ("*foo*bar*", "XXXfooYYYbarZZZ", true),
            ("*foo*bar*", "foobar", true),
            ("**", "anything", true),
            ("a*b*c", "abc", true),
            ("a*b*c", "aXXXbYYYc", true),
        ];

        // ACT & ASSERT
        for (pattern, text, expected) in test_cases {
            assert_eq!(glob_match(pattern, text), expected);
        }
    }

    #[test]
    fn test_glob_star_and_question_combined() {
        // ARRANGE
        let test_cases = vec![
            ("a?c*", "abcdef", true),
            ("*?c", "abc", true),
            ("a*?", "ab", true),
            ("a*?", "abcd", true),
            ("a*?", "a", false),
        ];

        // ACT & ASSERT
        for (pattern, text, expected) in test_cases {
            assert_eq!(glob_match(pattern, text), expected);
        }
    }

    #[test]
    fn test_glob_consecutive_questions() {
        // ARRANGE
        let test_cases = vec![
            ("???", "abc", true),
            ("???", "ab", false),
            ("???", "abcd", false),
        ];

        // ACT & ASSERT
        for (pattern, text, expected) in test_cases {
            assert_eq!(glob_match(pattern, text), expected);
        }
    }

    #[test]
    fn test_glob_trailing_star() {
        // ARRANGE
        let test_cases = vec![
            ("foo*", "foo", true),
            ("foo*", "foobar", true),
            ("foo*", "foo123456789", true),
        ];

        // ACT & ASSERT
        for (pattern, text, expected) in test_cases {
            assert_eq!(glob_match(pattern, text), expected);
        }
    }

    #[test]
    fn test_glob_leading_star() {
        // ARRANGE
        let test_cases = vec![
            ("*foo", "foo", true),
            ("*foo", "barfoo", true),
            ("*foo", "foobar", false),
        ];

        // ACT & ASSERT
        for (pattern, text, expected) in test_cases {
            assert_eq!(glob_match(pattern, text), expected);
        }
    }

    #[test]
    fn test_glob_backtracking() {
        // ARRANGE
        let test_cases = vec![
            ("a*a", "aa", true),
            ("a*a", "aXa", true),
            ("a*a", "aXXXa", true),
            ("*a*a*a*", "aaa", true),
            ("*a*a*a*", "XaYaZaW", true),
        ];

        // ACT & ASSERT
        for (pattern, text, expected) in test_cases {
            assert_eq!(glob_match(pattern, text), expected);
        }
    }

    #[test]
    fn test_pci_modalias() {
        // ARRANGE
        let pattern = "pci:v00008086d00001521sv*sd*bc*sc*i*";
        let test_cases = vec![
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
    fn test_intel_i226v_modalias() {
        // ARRANGE
        let pattern = "pci:v00008086d0000125Csv*sd*bc*sc*i*";
        let modalias = "pci:v00008086d0000125Csv00001043sd000087D2bc02sc00i00";
        let modalias_lower = "pci:v00008086d0000125csv00001043sd000087d2bc02sc00i00";

        // ACT & ASSERT
        assert!(glob_match(pattern, modalias));

        assert!(!glob_match(pattern, modalias_lower));

        assert!(glob_match_icase(
            pattern,
            &modalias_lower.to_ascii_lowercase()
        ));
    }

    #[test]
    fn test_usb_modalias() {
        // ARRANGE
        let pattern = "usb:v*p*d*dc*dsc*dp*ic03isc01ip01*";
        let modalias = "usb:v046DpC52Bd2111dc00dsc00dp00ic03isc01ip01in00";

        // ACT & ASSERT
        assert!(glob_match(pattern, modalias));
    }

    #[test]
    fn test_acpi_modalias() {
        // ARRANGE
        let pattern = "acpi:ACPI0003:";
        let test_cases = vec![("acpi:ACPI0003:", true), ("acpi:ACPI0004:", false)];

        // ACT & ASSERT
        for (modalias, expected) in test_cases {
            assert_eq!(glob_match(pattern, modalias), expected);
        }
    }

    #[test]
    fn test_parse_alias_line_valid() {
        // ACT & ASSERT
        let result = parse_alias_line("alias pci:v00008086d* igb");
        assert_eq!(
            result,
            Some(("pci:v00008086d*".to_string(), "igb".to_string()))
        );
    }

    #[test]
    fn test_parse_alias_line_with_extra_spaces() {
        // ACT & ASSERT
        let result = parse_alias_line("  alias   pci:pattern   module_name  ");
        assert_eq!(
            result,
            Some(("pci:pattern".to_string(), "module_name".to_string()))
        );
    }

    #[test]
    fn test_parse_alias_line_empty() {
        // ACT & ASSERT
        assert_eq!(parse_alias_line(""), None);
        assert_eq!(parse_alias_line("   "), None);
    }

    #[test]
    fn test_parse_alias_line_comment() {
        // ACT & ASSERT
        assert_eq!(parse_alias_line("# this is a comment"), None);
        assert_eq!(parse_alias_line("  # indented comment"), None);
    }

    #[test]
    fn test_parse_alias_line_no_alias_prefix() {
        // ACT & ASSERT
        assert_eq!(parse_alias_line("not an alias line"), None);
        assert_eq!(parse_alias_line("alias_not_right pattern module"), None);
    }

    #[test]
    fn test_parse_alias_line_missing_module() {
        // ACT & ASSERT
        assert_eq!(parse_alias_line("alias pattern_only"), None);
    }

    #[test]
    fn test_parse_alias_line_complex_pattern() {
        // ACT & ASSERT
        let result = parse_alias_line("alias pci:v00008086d0000125Csv*sd*bc*sc*i* igc");
        assert_eq!(
            result,
            Some((
                "pci:v00008086d0000125Csv*sd*bc*sc*i*".to_string(),
                "igc".to_string()
            ))
        );
    }

    #[test]
    fn test_alias_db_case_insensitive() {
        // ARRANGE
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        writeln!(file, "alias pci:v00008086d0000125Csv*sd*bc*sc*i* igc").expect("write failed");

        let db = AliasDb::load(file.path()).expect("load failed");
        let modalias = "pci:v00008086d0000125csv00001043sd000087d2bc02sc00i00";

        // ACT
        let result = db.find_module(modalias);

        // ASSERT
        assert_eq!(result, Some("igc"));
    }

    #[test]
    fn test_alias_db_empty_file() {
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
    fn test_alias_db_with_comments_and_blanks() {
        // ARRANGE
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        writeln!(file, "# Comment line").expect("write failed");
        writeln!(file, "").expect("write failed");
        writeln!(file, "alias pattern1 module1").expect("write failed");
        writeln!(file, "  # Another comment").expect("write failed");
        writeln!(file, "alias pattern2 module2").expect("write failed");

        // ACT
        let db = AliasDb::load(file.path()).expect("load failed");

        // ASSERT
        assert_eq!(db.len(), 2);
        assert_eq!(db.find_module("pattern1"), Some("module1"));
        assert_eq!(db.find_module("pattern2"), Some("module2"));
    }

    #[test]
    fn test_alias_db_multiple_entries() {
        // ARRANGE
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        writeln!(file, "alias pci:v00008086d00001521* igb").expect("write failed");
        writeln!(file, "alias pci:v00008086d0000125C* igc").expect("write failed");
        writeln!(file, "alias pci:v000010DE* nvidia").expect("write failed");

        let db = AliasDb::load(file.path()).expect("load failed");

        // ACT & ASSERT
        assert_eq!(db.len(), 3);
        assert!(!db.is_empty());

        assert_eq!(db.find_module("pci:v00008086d00001521sv1234"), Some("igb"));
        assert_eq!(db.find_module("pci:v00008086d0000125csv1234"), Some("igc"));
        assert_eq!(db.find_module("pci:v000010deABCD"), Some("nvidia"));
        assert_eq!(db.find_module("pci:v00001234d5678"), None);
    }

    #[test]
    fn test_alias_db_first_match_wins() {
        // ARRANGE
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        writeln!(file, "alias pci:* first_module").expect("write failed");
        writeln!(file, "alias pci:v00008086* second_module").expect("write failed");

        let db = AliasDb::load(file.path()).expect("load failed");

        // ACT
        let result = db.find_module("pci:v00008086d1234");

        // ASSERT
        assert_eq!(result, Some("first_module"));
    }

    #[test]
    fn test_alias_db_load_nonexistent_file() {
        // ACT & ASSERT
        let result = AliasDb::load(Path::new("/nonexistent/path/modules.alias"));
        assert!(result.is_err());
    }

    #[test]
    fn test_alias_db_real_world_patterns() {
        // ARRANGE
        let mut file = NamedTempFile::new().expect("Failed to create temp file");
        writeln!(file, "alias usb:v*p*d*dc*dsc*dp*ic03isc01ip01* usbhid").expect("write failed");
        writeln!(file, "alias acpi*:ACPI0003:* ac").expect("write failed");
        writeln!(file, "alias platform:efi-framebuffer efifb").expect("write failed");

        let db = AliasDb::load(file.path()).expect("load failed");

        // ACT & ASSERT
        assert_eq!(
            db.find_module("usb:v046dpC52bd2111dc00dsc00dp00ic03isc01ip01in00"),
            Some("usbhid")
        );
        assert_eq!(db.find_module("acpi:acpi0003:"), Some("ac"));
        assert_eq!(db.find_module("platform:efi-framebuffer"), Some("efifb"));
    }
}
