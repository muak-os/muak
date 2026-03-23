//! Parser and matcher for SELinux `file_contexts` produced by `secilc -f`.

use std::io::{BufRead, BufReader, Read};

use crate::error::{ErofsError, Result};

/// Parsed file-contexts database mapping path patterns to SELinux labels.
#[derive(Debug, Clone)]
pub struct FileContexts {
    exact: Vec<(String, String)>,
    prefix: Vec<(String, String)>,
    default: Option<String>,
}

impl FileContexts {
    /// Parse a `file_contexts` file from a reader.
    pub fn from_reader(r: impl Read) -> Result<Self> {
        let reader = BufReader::new(r);
        let mut exact = Vec::new();
        let mut prefix = Vec::new();
        let mut default = None;

        for line in reader.lines() {
            let line = line.map_err(ErofsError::Io)?;
            Self::parse_entry(line.trim(), &mut exact, &mut prefix, &mut default)?;
        }

        prefix.sort_by(|a, b| b.0.len().cmp(&a.0.len()));

        Ok(Self {
            exact,
            prefix,
            default,
        })
    }

    fn parse_entry(
        line: &str,
        exact: &mut Vec<(String, String)>,
        prefix: &mut Vec<(String, String)>,
        default: &mut Option<String>,
    ) -> Result<()> {
        if line.is_empty() || line.starts_with('#') {
            return Ok(());
        }
        let (pattern, context) = parse_line(line)?;
        if context == "<<none>>" {
            return Ok(());
        }
        match categorize_pattern(&pattern) {
            PatternKind::Default => *default = Some(context),
            PatternKind::Exact(path) => exact.push((path, context)),
            PatternKind::Prefix(pfx) => prefix.push((pfx, context)),
        }
        Ok(())
    }

    /// Look up the SELinux label for a given absolute path.
    pub fn label_for(&self, path: &str) -> Option<&str> {
        self.exact
            .iter()
            .find(|(p, _)| p == path)
            .or_else(|| {
                self.prefix
                    .iter()
                    .find(|(pfx, _)| path.starts_with(pfx.as_str()))
            })
            .map(|(_, ctx)| ctx.as_str())
            .or(self.default.as_deref())
    }
}

enum PatternKind {
    Default,
    Exact(String),
    Prefix(String),
}

/// Categorize a pattern into exact, prefix, or catch-all default.
fn categorize_pattern(pattern: &str) -> PatternKind {
    if pattern == "/.*" {
        return PatternKind::Default;
    }
    if let Some(pfx) = pattern.strip_suffix("/.*") {
        PatternKind::Prefix(format!("{pfx}/"))
    } else if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') {
        PatternKind::Prefix(strip_glob_tail(pattern))
    } else {
        PatternKind::Exact(pattern.to_string())
    }
}

/// Strip trailing glob characters to extract the fixed prefix.
fn strip_glob_tail(pattern: &str) -> String {
    let end = pattern.find(['*', '?', '[']).unwrap_or(pattern.len());
    pattern[..end].to_string()
}

/// Parse a single file_contexts line into (pattern, context).
fn parse_line(line: &str) -> Result<(String, String)> {
    let mut parts = line.split_whitespace();
    let pattern = parts
        .next()
        .ok_or_else(|| ErofsError::FileContexts("empty line".to_string()))?;
    let rest: Vec<&str> = parts.collect();
    let context = match rest.len() {
        1 => rest[0],
        2 => rest[1],
        _ => {
            return Err(ErofsError::FileContexts(format!(
                "unexpected format: {line}"
            )));
        }
    };
    Ok((pattern.to_string(), context.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_fc(input: &str) -> FileContexts {
        FileContexts::from_reader(input.as_bytes()).expect("parse")
    }

    #[test]
    fn exact_match_takes_priority() {
        let fc = make_fc(
            "/.*    system_u:object_r:file_t:s0\n\
             /sbin/init    system_u:object_r:granola_exec_t:s0\n",
        );
        assert_eq!(
            fc.label_for("/sbin/init"),
            Some("system_u:object_r:granola_exec_t:s0")
        );
    }

    #[test]
    fn default_label_for_unmatched_path() {
        let fc = make_fc("/.*    system_u:object_r:file_t:s0\n");
        assert_eq!(
            fc.label_for("/anything"),
            Some("system_u:object_r:file_t:s0")
        );
    }

    #[test]
    fn prefix_match() {
        let fc = make_fc(
            "/.*    system_u:object_r:file_t:s0\n\
             /lib/modules/.*    system_u:object_r:modules_t:s0\n",
        );
        assert_eq!(
            fc.label_for("/lib/modules/foo.ko"),
            Some("system_u:object_r:modules_t:s0")
        );
    }

    #[test]
    fn exact_match_over_prefix() {
        let fc = make_fc(
            "/lib/modules/.*    system_u:object_r:modules_t:s0\n\
             /lib/modules    system_u:object_r:modules_t:s0\n",
        );
        assert_eq!(
            fc.label_for("/lib/modules"),
            Some("system_u:object_r:modules_t:s0")
        );
    }

    #[test]
    fn longer_prefix_wins() {
        let fc = make_fc(
            "/a/.*    ctx_a\n\
             /a/b/.*    ctx_ab\n",
        );
        assert_eq!(fc.label_for("/a/b/c"), Some("ctx_ab"));
    }

    #[test]
    fn none_context_skipped() {
        let fc = make_fc(
            "/.*    system_u:object_r:file_t:s0\n\
             /skip    <<none>>\n",
        );
        assert_eq!(fc.label_for("/skip"), Some("system_u:object_r:file_t:s0"));
    }

    #[test]
    fn no_match_returns_none() {
        let fc = make_fc("/sbin/init    system_u:object_r:granola_exec_t:s0\n");
        assert_eq!(fc.label_for("/other"), None);
    }

    #[test]
    fn empty_input() {
        let fc = make_fc("");
        assert_eq!(fc.label_for("/anything"), None);
    }

    #[test]
    fn comments_and_blank_lines_ignored() {
        let fc = make_fc(
            "# comment\n\
             \n\
             /.*    system_u:object_r:file_t:s0\n",
        );
        assert_eq!(fc.label_for("/foo"), Some("system_u:object_r:file_t:s0"));
    }

    #[test]
    fn file_type_column_handled() {
        let fc = make_fc(
            "/sbin/init    --    system_u:object_r:granola_exec_t:s0\n\
             /.*    system_u:object_r:file_t:s0\n",
        );
        assert_eq!(
            fc.label_for("/sbin/init"),
            Some("system_u:object_r:granola_exec_t:s0")
        );
    }

    #[test]
    fn root_path_gets_default() {
        // ARRANGE
        let fc = make_fc("/.*    system_u:object_r:file_t:s0\n");

        // ACT & ASSERT
        assert_eq!(fc.label_for("/"), Some("system_u:object_r:file_t:s0"));
    }

    #[test]
    fn glob_question_mark_pattern_treated_as_prefix() {
        // ARRANGE
        let fc = make_fc("/bin/b?sh    system_u:object_r:shell_exec_t:s0\n");

        // ACT & ASSERT
        assert_eq!(
            fc.label_for("/bin/bash"),
            Some("system_u:object_r:shell_exec_t:s0")
        );
    }

    #[test]
    fn glob_bracket_pattern_treated_as_prefix() {
        // ARRANGE
        let fc = make_fc("/lib/lib[a-z]*    system_u:object_r:lib_t:s0\n");

        // ACT & ASSERT
        assert_eq!(
            fc.label_for("/lib/libfoo.so"),
            Some("system_u:object_r:lib_t:s0")
        );
    }

    #[test]
    fn parse_line_too_many_fields_returns_error() {
        // ARRANGE
        let input = "/path  --  ctx  extra\n";

        // ACT
        let result = FileContexts::from_reader(input.as_bytes());

        // ASSERT
        assert!(result.is_err());
    }

    #[test]
    fn strip_glob_tail_no_glob_returns_full_string() {
        // ARRANGE
        let pattern = "/etc/passwd";

        // ACT
        let result = strip_glob_tail(pattern);

        // ASSERT
        assert_eq!(result, "/etc/passwd");
    }

    #[test]
    fn strip_glob_tail_at_start_returns_empty() {
        // ARRANGE
        let pattern = "*foo";

        // ACT
        let result = strip_glob_tail(pattern);

        // ASSERT
        assert_eq!(result, "");
    }
}
