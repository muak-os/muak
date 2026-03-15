//! Loads service definitions from files on disk.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};

use crate::supervisor::Service;

const SERVICES_DIR: &str = "/etc/services";

/// Reads all service files in `dir` and returns their parsed contents.
pub fn scan_services(dir: &Path) -> Result<Vec<Service>> {
    let mut files = Vec::new();
    let read_dir = std::fs::read_dir(dir)
        .with_context(|| format!("Failed to read services directory: {}", dir.display()))?;
    for entry in read_dir {
        let entry = entry.with_context(|| "Failed to read directory entry")?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read service file: {}", path.display()))?;
        let svc: Service = toml::from_str(&content)
            .with_context(|| format!("Failed to parse service file: {}", path.display()))?;
        files.push(svc);
    }
    Ok(files)
}

/// Substitutes `$VAR` patterns in `s` using `env`.
pub fn substitute_vars(s: &str, env: &HashMap<&str, &str>) -> String {
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            result.push(bytes[i] as char);
            i += 1;
            continue;
        }
        i += 1;
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        let var = &s[start..i];
        match env.get(var) {
            Some(&val) => result.push_str(val),
            None => {
                result.push('$');
                result.push_str(var);
            }
        }
    }
    result
}

/// Detects cycles in the dependency graph via DFS. Returns an error naming the cycle.
fn detect_cycles(index: &HashMap<String, &Service>) -> Result<()> {
    let mut visited: HashSet<&str> = HashSet::new();
    let mut stack: Vec<&str> = Vec::new();
    for name in index.keys() {
        if !visited.contains(name.as_str()) {
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
    let index: HashMap<String, &Service> = defs.iter().map(|s| (s.name.clone(), s)).collect();

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
        let mut m = HashMap::new();
        m.insert("PORT", "50051");
        m.insert("MODE", "normal");
        m
    }

    #[test]
    fn substitute_known_var() {
        // ARRANGE
        let env = env();

        // ACT
        let result = substitute_vars("--port $PORT", &env);

        // ASSERT
        assert_eq!(result, "--port 50051");
    }

    #[test]
    fn substitute_unknown_var_preserved() {
        // ARRANGE
        let env = env();

        // ACT
        let result = substitute_vars("$UNKNOWN", &env);

        // ASSERT
        assert_eq!(result, "$UNKNOWN");
    }

    #[test]
    fn substitute_multiple_vars() {
        // ARRANGE
        let env = env();

        // ACT
        let result = substitute_vars("/sbin/apid --port $PORT --mode $MODE", &env);

        // ASSERT
        assert_eq!(result, "/sbin/apid --port 50051 --mode normal");
    }

    fn make_svc(name: &str, cmd: &str, deps: Vec<&str>) -> Service {
        Service {
            name: name.to_string(),
            command: cmd.to_string(),
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn prepare_applies_var_substitution() {
        // ARRANGE
        let defs = vec![make_svc("apid", "/sbin/apid --port $PORT", vec![])];
        let env = env();

        // ACT
        let svcs = prepare(defs, &env).unwrap();

        // ASSERT
        assert_eq!(svcs[0].command, "/sbin/apid --port 50051");
    }

    #[test]
    fn prepare_returns_all_services() {
        // ARRANGE
        let defs = vec![
            make_svc("a", "/bin/a", vec![]),
            make_svc("b", "/bin/b", vec!["a"]),
            make_svc("c", "/bin/c", vec![]),
        ];
        let env = HashMap::new();

        // ACT
        let svcs = prepare(defs, &env).unwrap();

        // ASSERT
        let mut names: Vec<_> = svcs.iter().map(|s| s.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn prepare_detects_cycle() {
        // ARRANGE
        let defs = vec![
            make_svc("a", "/bin/a", vec!["b"]),
            make_svc("b", "/bin/b", vec!["a"]),
        ];
        let env = HashMap::new();

        // ACT + ASSERT
        assert!(prepare(defs, &env).is_err());
    }

    #[test]
    fn scan_services_reads_files() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let content = r#"name = "testsvc"
command = "/bin/test"
depends_on = []
"#;
        std::fs::write(dir.path().join("testsvc.toml"), content).unwrap();
        std::fs::write(dir.path().join("ignored.txt"), "noise").unwrap();

        // ACT
        let svcs = scan_services(dir.path()).unwrap();

        // ASSERT
        assert_eq!(svcs.len(), 1);
        assert_eq!(svcs[0].name, "testsvc");
    }
}
