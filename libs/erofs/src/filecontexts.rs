//! Parser and matcher for `SELinux` `file_contexts` data produced by `secilc -f`.

use core::cmp::Reverse;
use std::io::{BufRead as _, BufReader, Read};

use crate::error::{ErofsError, Result};

/// Parsed `file_contexts` database mapping path patterns to `SELinux` labels.
#[derive(Debug, Clone)]
pub struct FileContexts {
    exact: Vec<(String, String)>,
    prefix: Vec<(String, String)>,
    default: Option<String>,
}

impl FileContexts {
    /// Parse a `file_contexts` file from a reader.
    ///
    /// # Errors
    ///
    /// Returns an error when a line cannot be read or parsed.
    pub fn from_reader<R: Read>(reader: R) -> Result<Self> {
        let reader = BufReader::new(reader);
        let mut exact = Vec::new();
        let mut prefix = Vec::new();
        let mut default = None;

        for line in reader.lines() {
            let line = line.map_err(ErofsError::Io)?;
            Self::parse_entry(line.trim(), &mut exact, &mut prefix, &mut default)?;
        }

        prefix.sort_by_key(|prefix_entry| Reverse(prefix_entry.0.len()));

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

    /// Look up the `SELinux` label for the given absolute `path`.
    #[must_use]
    pub fn label_for(&self, path: &str) -> Option<&str> {
        self.exact
            .iter()
            .find(|entry| entry.0 == path)
            .or_else(|| {
                self.prefix
                    .iter()
                    .find(|entry| path.starts_with(entry.0.as_str()))
            })
            .map(|entry| entry.1.as_str())
            .or(self.default.as_deref())
    }
}

/// Categorization of a `file_contexts` pattern.
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
        PatternKind::Exact(pattern.to_owned())
    }
}

/// Strip trailing glob characters to extract the fixed prefix.
fn strip_glob_tail(pattern: &str) -> String {
    let end = pattern.find(['*', '?', '[']).unwrap_or(pattern.len());
    pattern.get(..end).unwrap_or_default().to_owned()
}

/// Parse a single `file_contexts` line into `(pattern, context)`.
fn parse_line(line: &str) -> Result<(String, String)> {
    let mut parts = line.split_whitespace();
    let pattern = parts
        .next()
        .ok_or_else(|| ErofsError::FileContexts("empty line".to_owned()))?;
    let rest: Vec<&str> = parts.collect();
    let (first, second, third) = (rest.first().copied(), rest.get(1).copied(), rest.get(2));
    let Some(context) = (match (first, second, third) {
        (Some(context), None, None) | (_, Some(context), None) => Some(context),
        _ => None,
    }) else {
        return Err(ErofsError::FileContexts(format!(
            "unexpected format: {line}"
        )));
    };
    Ok((pattern.to_owned(), context.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_fc(input: &str) -> FileContexts {
        FileContexts::from_reader(input.as_bytes()).expect("parse")
    }

    #[test]
    fn exact_match_takes_priority() {
        // ARRANGE
        let fc = make_fc(
            "/.*    system_u:object_r:file_t:s0\n\
             /sbin/init    system_u:object_r:granola_exec_t:s0\n",
        );
        // ACT
        // ASSERT
        assert_eq!(
            fc.label_for("/sbin/init"),
            Some("system_u:object_r:granola_exec_t:s0")
        );
    }

    #[test]
    fn default_label_for_unmatched_path() {
        // ARRANGE
        let fc = make_fc("/.*    system_u:object_r:file_t:s0\n");
        // ACT
        // ASSERT
        assert_eq!(
            fc.label_for("/anything"),
            Some("system_u:object_r:file_t:s0")
        );
    }

    #[test]
    fn prefix_match() {
        // ARRANGE
        let fc = make_fc(
            "/.*    system_u:object_r:file_t:s0\n\
             /lib/modules/.*    system_u:object_r:modules_t:s0\n",
        );
        // ACT
        // ASSERT
        assert_eq!(
            fc.label_for("/lib/modules/foo.ko"),
            Some("system_u:object_r:modules_t:s0")
        );
    }

    #[test]
    fn exact_match_over_prefix() {
        // ARRANGE
        let fc = make_fc(
            "/lib/modules/.*    system_u:object_r:modules_t:s0\n\
             /lib/modules    system_u:object_r:modules_t:s0\n",
        );
        // ACT
        // ASSERT
        assert_eq!(
            fc.label_for("/lib/modules"),
            Some("system_u:object_r:modules_t:s0")
        );
    }

    #[test]
    fn longer_prefix_wins() {
        // ARRANGE
        let fc = make_fc(
            "/a/.*    ctx_a\n\
             /a/b/.*    ctx_ab\n",
        );
        // ACT
        // ASSERT
        assert_eq!(fc.label_for("/a/b/c"), Some("ctx_ab"));
    }

    #[test]
    fn none_context_skipped() {
        // ARRANGE
        let fc = make_fc(
            "/.*    system_u:object_r:file_t:s0\n\
             /skip    <<none>>\n",
        );
        // ACT
        // ASSERT
        assert_eq!(fc.label_for("/skip"), Some("system_u:object_r:file_t:s0"));
    }

    #[test]
    fn no_match_returns_none() {
        // ARRANGE
        let fc = make_fc("/sbin/init    system_u:object_r:granola_exec_t:s0\n");
        // ACT
        // ASSERT
        assert_eq!(fc.label_for("/other"), None);
    }

    #[test]
    fn empty_input() {
        // ARRANGE
        let fc = make_fc("");
        // ACT
        // ASSERT
        assert_eq!(fc.label_for("/anything"), None);
    }

    #[test]
    fn comments_and_blank_lines_ignored() {
        // ARRANGE
        let fc = make_fc(
            "# comment\n\
             \n\
             /.*    system_u:object_r:file_t:s0\n",
        );
        // ACT
        // ASSERT
        assert_eq!(fc.label_for("/foo"), Some("system_u:object_r:file_t:s0"));
    }

    #[test]
    fn file_type_column_handled() {
        // ARRANGE
        let fc = make_fc(
            "/sbin/init    --    system_u:object_r:granola_exec_t:s0\n\
             /.*    system_u:object_r:file_t:s0\n",
        );
        // ACT
        // ASSERT
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
        // ACT
        // ASSERT
        assert_eq!(fc.label_for("/"), Some("system_u:object_r:file_t:s0"));
    }

    #[test]
    fn glob_question_mark_pattern_treated_as_prefix() {
        // ARRANGE
        let fc = make_fc("/bin/b?sh    system_u:object_r:shell_exec_t:s0\n");

        // ACT & ASSERT
        // ACT
        // ASSERT
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
        // ACT
        // ASSERT
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
        result.unwrap_err();
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
