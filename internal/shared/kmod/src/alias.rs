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
        assert!(glob_match("foo", "foo"));
        assert!(!glob_match("foo", "bar"));
        assert!(!glob_match("foo", "foobar"));
    }

    #[test]
    fn test_glob_star() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("foo*", "foobar"));
        assert!(glob_match("*bar", "foobar"));
        assert!(glob_match("foo*bar", "fooXXXbar"));
        assert!(glob_match("foo*bar", "foobar"));
        assert!(!glob_match("foo*bar", "foobaz"));
    }

    #[test]
    fn test_glob_question() {
        assert!(glob_match("fo?", "foo"));
        assert!(glob_match("f??", "foo"));
        assert!(!glob_match("fo?", "fo"));
        assert!(!glob_match("fo?", "fooo"));
    }

    #[test]
    fn test_pci_modalias() {
        let pattern = "pci:v00008086d00001521sv*sd*bc*sc*i*";
        assert!(glob_match(
            pattern,
            "pci:v00008086d00001521sv00001028sd00000001bc02sc00i00"
        ));
        assert!(!glob_match(
            pattern,
            "pci:v00008086d00001522sv00001028sd00000001bc02sc00i00"
        ));
    }

    #[test]
    fn test_intel_i226v_modalias() {
        let pattern = "pci:v00008086d0000125Csv*sd*bc*sc*i*";
        let modalias = "pci:v00008086d0000125Csv00001043sd000087D2bc02sc00i00";
        assert!(glob_match(pattern, modalias));

        let modalias_lower = "pci:v00008086d0000125csv00001043sd000087d2bc02sc00i00";
        assert!(!glob_match(pattern, modalias_lower));

        assert!(glob_match_icase(
            pattern,
            &modalias_lower.to_ascii_lowercase()
        ));
    }

    #[test]
    fn test_alias_db_case_insensitive() {
        use std::io::Write;

        let dir = std::env::temp_dir();
        let path = dir.join("test_modules_alias");
        {
            let mut file = std::fs::File::create(&path).unwrap();
            writeln!(file, "alias pci:v00008086d0000125Csv*sd*bc*sc*i* igc").unwrap();
        }

        let db = AliasDb::load(&path).unwrap();
        std::fs::remove_file(&path).ok();

        let modalias = "pci:v00008086d0000125csv00001043sd000087d2bc02sc00i00";
        assert_eq!(db.find_module(modalias), Some("igc"));
    }
}
