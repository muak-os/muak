use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Debug)]
pub struct AliasDb {
    entries: Vec<(String, String)>, // (pattern, module)
}

impl AliasDb {
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut entries = Vec::new();

        for line in reader.lines() {
            let line = line?;
            let line = line.trim();

            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let Some(rest) = line.strip_prefix("alias ") else {
                continue;
            };
            let Some((pattern, module)) = rest.rsplit_once(' ') else {
                continue;
            };
            entries.push((pattern.trim().to_string(), module.trim().to_string()));
        }

        Ok(Self { entries })
    }

    pub fn find_module(&self, modalias: &str) -> Option<&str> {
        for (pattern, module) in &self.entries {
            if glob_match(pattern, modalias) {
                return Some(module);
            }
        }
        None
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();

    let mut p = 0;
    let mut t = 0;
    let mut star_p = None;
    let mut star_t = None;

    while t < text.len() {
        if p < pattern.len() {
            match pattern[p] {
                b'*' => {
                    star_p = Some(p);
                    star_t = Some(t);
                    p += 1;
                    continue;
                }
                b'?' => {
                    p += 1;
                    t += 1;
                    continue;
                }
                c if c == text[t] => {
                    p += 1;
                    t += 1;
                    continue;
                }
                _ => {}
            }
        }

        if let (Some(sp), Some(st)) = (star_p, star_t) {
            p = sp + 1;
            star_t = Some(st + 1);
            t = st + 1;
        } else {
            return false;
        }
    }

    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }

    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
