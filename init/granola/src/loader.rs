//! Loads service definitions from files on disk.

use std::collections::{HashMap, HashSet};
use std::iter::Peekable;
use std::path::Path;

use anyhow::{Context as _, Result};

use crate::supervisor::service::Service;

const SERVICES_DIR: &str = "/etc/services";

/// Reads all service files in `dir` and returns their parsed contents.
pub fn scan_services(dir: &Path) -> Result<Vec<Service>> {
    let mut files = Vec::new();
    let read_dir =
        std::fs::read_dir(dir).with_context(|| format!("Failed to read dir: {}", dir.display()))?;
    for entry in read_dir {
        let entry = entry.with_context(|| "Failed to read directory entry")?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("service") {
            continue;
        }
        if let Some(svc) = read_service_file(&path) {
            files.push(svc);
        }
    }
    Ok(files)
}

/// Reads a single service file, logging (and skipping) unreadable or malformed files.
fn read_service_file(path: &Path) -> Option<Service> {
    let content = std::fs::read_to_string(path)
        .inspect_err(|error| {
            kmsg::warn!("Failed to read service file {}: {error}", path.display());
        })
        .ok()?;
    let svc = toml::from_str::<Service>(&content)
        .inspect_err(|error| {
            kmsg::warn!("Malformed service file {}: {error}", path.display());
        })
        .ok()?;
    Some(svc)
}

/// Substitutes `$VAR` patterns in `text` using `env`.
pub fn substitute_vars(text: &str, env: &HashMap<&str, &str>) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '$' => append_variable(&mut chars, env, &mut result),
            plain => result.push(plain),
        }
    }
    result
}

/// Consumes a variable name from `chars` and appends its value (or the raw
/// `$NAME` reference when unknown) to `result`.
fn append_variable(
    chars: &mut Peekable<std::str::Chars<'_>>,
    env: &HashMap<&str, &str>,
    result: &mut String,
) {
    let mut name = String::new();
    while let Some(&ch) = chars.peek()
        && (ch.is_ascii_alphanumeric() || ch == '_')
    {
        name.push(ch);
        chars.next();
    }

    if let Some(&value) = env.get(name.as_str()) {
        result.push_str(value);
        return;
    }
    result.push('$');
    result.push_str(&name);
}

/// Detects cycles in the dependency graph via DFS. Returns an error naming the cycle.
fn detect_cycles(index: &HashMap<String, &Service>) -> Result<()> {
    let mut visited: HashSet<&str> = HashSet::new();
    let mut stack: Vec<&str> = Vec::new();
    let mut names: Vec<&str> = index.keys().map(String::as_str).collect();
    names.sort_unstable();

    for name in names {
        if !visited.contains(name) {
            dfs_cycle(name, index, &mut visited, &mut stack)?;
        }
    }
    Ok(())
}

fn dfs_cycle<'a>(
    name: &'a str,
    index: &'a HashMap<String, &'a Service>,
    visited: &mut HashSet<&'a str>,
    stack: &mut Vec<&'a str>,
) -> Result<()> {
    use anyhow::bail;
    if stack.contains(&name) {
        bail!(
            "Dependency cycle detected: {} -> {}",
            stack.join(" -> "),
            name
        );
    }
    if visited.contains(name) {
        return Ok(());
    }
    stack.push(name);
    if let Some(svc) = index.get(name) {
        for dep in &svc.depends_on {
            dfs_cycle(dep.as_str(), index, visited, stack)?;
        }
    }
    stack.pop();
    visited.insert(name);
    Ok(())
}

/// Applies env var substitution to all services and validates the dependency graph.
pub fn prepare(defs: Vec<Service>, env: &HashMap<&str, &str>) -> Result<Vec<Service>> {
    let index: HashMap<String, &Service> = defs.iter().map(|svc| (svc.name.clone(), svc)).collect();

    detect_cycles(&index)?;

    defs.into_iter()
        .map(|svc| {
            Ok(Service {
                name: svc.name,
                command: substitute_vars(&svc.command, env),
                depends_on: svc.depends_on,
            })
        })
        .collect()
}

/// Convenience entry-point: scans all service files and prepares them.
pub fn load(env: &HashMap<&str, &str>) -> Result<Vec<Service>> {
    let defs = scan_services(Path::new(SERVICES_DIR)).context("Failed to scan service files")?;
    prepare(defs, env)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> HashMap<&'static str, &'static str> {
        let mut vars = HashMap::new();
        vars.insert("PORT", "50051");
        vars.insert("MODE", "normal");
        vars
    }

    #[test]
    fn substitute_known_var() {
        // ARRANGE
        let vars = env();

        // ACT
        let result = substitute_vars("--port $PORT", &vars);

        // ASSERT
        assert_eq!(result, "--port 50051");
    }

    #[test]
    fn substitute_unknown_var_preserved() {
        // ARRANGE
        let vars = env();

        // ACT
        let result = substitute_vars("$UNKNOWN", &vars);

        // ASSERT
        assert_eq!(result, "$UNKNOWN");
    }

    #[test]
    fn substitute_multiple_vars() {
        // ARRANGE
        let vars = env();

        // ACT
        let result = substitute_vars("/sbin/apid --port $PORT --mode $MODE", &vars);

        // ASSERT
        assert_eq!(result, "/sbin/apid --port 50051 --mode normal");
    }

    fn make_svc(name: &str, cmd: &str, deps: &[&str]) -> Service {
        Service {
            name: name.to_owned(),
            command: cmd.to_owned(),
            depends_on: deps.iter().copied().map(str::to_owned).collect(),
        }
    }

    #[test]
    fn prepare_applies_var_substitution() {
        // ARRANGE
        let defs = vec![make_svc("apid", "/sbin/apid --port $PORT", &[])];
        let vars = env();

        // ACT
        let svcs = prepare(defs, &vars).unwrap();

        // ASSERT
        assert_eq!(
            svcs.first().map(|svc| svc.command.as_str()),
            Some("/sbin/apid --port 50051")
        );
    }

    #[test]
    fn prepare_returns_all_services() {
        // ARRANGE
        let defs = vec![
            make_svc("a", "/bin/a", &[]),
            make_svc("b", "/bin/b", &["a"]),
            make_svc("c", "/bin/c", &[]),
        ];
        let vars = HashMap::new();

        // ACT
        let svcs = prepare(defs, &vars).unwrap();

        // ASSERT
        let mut names: Vec<_> = svcs.iter().map(|svc| svc.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn prepare_detects_cycle() {
        // ARRANGE
        let defs = vec![
            make_svc("a", "/bin/a", &["b"]),
            make_svc("b", "/bin/b", &["a"]),
        ];
        let vars = HashMap::new();

        // ACT + ASSERT
        prepare(defs, &vars).expect_err("cycle should be detected");
    }

    #[test]
    fn scan_services_reads_files() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let content = r#"name = "testsvc"
command = "/bin/test"
depends_on = []
"#;
        std::fs::write(dir.path().join("testsvc.service"), content).unwrap();
        std::fs::write(dir.path().join("ignored.txt"), "noise").unwrap();

        // ACT
        let svcs = scan_services(dir.path()).unwrap();

        // ASSERT
        assert_eq!(svcs.len(), 1);
        assert_eq!(svcs.first().map(|svc| svc.name.as_str()), Some("testsvc"));
    }

    #[test]
    fn scan_services_skips_malformed_file() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let valid = r#"name = "good"
command = "/bin/good"
depends_on = []
"#;
        std::fs::write(dir.path().join("good.service"), valid).unwrap();
        std::fs::write(dir.path().join("bad.service"), "not valid ][[[").unwrap();

        // ACT
        let svcs = scan_services(dir.path()).unwrap();

        // ASSERT
        assert_eq!(svcs.len(), 1);
        assert_eq!(svcs.first().map(|svc| svc.name.as_str()), Some("good"));
    }

    #[test]
    fn substitute_vars_skips_malformed_variable_at_end() {
        // ARRANGE
        let vars = env();

        // ACT
        let result = substitute_vars("trailing $", &vars);

        // ASSERT
        assert_eq!(result, "trailing $");
    }

    #[test]
    fn substitute_vars_handles_multi_byte_characters() {
        // ARRANGE
        let vars = env();

        // ACT
        let result = substitute_vars("héllo $PORT wörld", &vars);

        // ASSERT
        assert_eq!(result, "héllo 50051 wörld");
    }

    #[test]
    fn substitute_vars_preserves_unknown_var_between_multi_byte_characters() {
        // ARRANGE
        let vars = HashMap::new();

        // ACT
        let result = substitute_vars("héllo $PORT wörld", &vars);

        // ASSERT
        assert_eq!(result, "héllo $PORT wörld");
    }
}
