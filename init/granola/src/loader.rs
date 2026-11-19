//! Loads service definitions from files on disk.

use std::collections::{HashMap, HashSet};
use std::iter::Peekable;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use sandbox::probe;

use crate::supervisor::service::Service;

const SERVICES_DIR: &str = "/etc/services";

/// Reads all service files in `dir` and returns their parsed contents.
pub(crate) fn scan_services(dir: &Path, caps: probe::Capabilities) -> Result<Vec<Service>> {
    let mut files = Vec::new();
    let read_dir =
        std::fs::read_dir(dir).with_context(|| format!("Failed to read dir: {}", dir.display()))?;
    for entry in read_dir {
        let entry = entry.with_context(|| "Failed to read directory entry")?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("service") {
            continue;
        }
        if let Some(svc) = read_service_file(&path, caps) {
            files.push(svc);
        }
    }

    Ok(files)
}

/// Reads a single service file, logging (and skipping) unreadable, malformed or unsandboxed files.
fn read_service_file(path: &Path, caps: probe::Capabilities) -> Option<Service> {
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
    if let Err(error) = validate(&svc, caps) {
        kmsg::error!("Refusing service {}: {error:#}", svc.name);
        return None;
    }

    Some(svc)
}

/// Checks that `service` carries a sandbox table this machine can honour.
fn validate(service: &Service, caps: probe::Capabilities) -> Result<()> {
    let sandbox = service
        .sandbox
        .as_ref()
        .context("missing [sandbox] table")?;
    let policy = sandbox.policy().context("invalid [sandbox] table")?;
    policy
        .resolve(&caps)
        .context("[sandbox] policy cannot be honoured on this machine")?;

    Ok(())
}

/// Substitutes `$VAR` patterns in `text` using `env`.
pub(crate) fn substitute_vars(text: &str, env: &HashMap<String, String>) -> String {
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

/// Consumes a variable name from `chars` and appends its value to `result`.
fn append_variable(
    chars: &mut Peekable<std::str::Chars<'_>>,
    env: &HashMap<String, String>,
    result: &mut String,
) {
    let mut name = String::new();
    while let Some(&ch) = chars.peek()
        && (ch.is_ascii_alphanumeric() || ch == '_')
    {
        name.push(ch);
        chars.next();
    }

    if let Some(value) = env.get(name.as_str()) {
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
pub(crate) fn prepare(defs: Vec<Service>, env: &HashMap<String, String>) -> Result<Vec<Service>> {
    let index: HashMap<String, &Service> = defs.iter().map(|svc| (svc.name.clone(), svc)).collect();

    detect_cycles(&index)?;

    defs.into_iter()
        .map(|svc| {
            Ok(Service {
                name: svc.name,
                command: substitute_vars(&svc.command, env),
                depends_on: svc.depends_on,
                sandbox: svc.sandbox,
            })
        })
        .collect()
}

/// Convenience entry-point: scans all service files and prepares them.
pub(crate) fn load(env: &HashMap<String, String>) -> Result<Vec<Service>> {
    let caps = probe::current();
    let defs =
        scan_services(Path::new(SERVICES_DIR), caps).context("Failed to scan service files")?;

    prepare(defs, env)
}

#[cfg(test)]
mod tests {
    use sandbox::cgroup;

    use super::*;

    const UNSANDBOXED: &str = r#"
[sandbox]
profile = "none"
"#;

    fn env() -> HashMap<String, String> {
        let mut vars = HashMap::new();
        vars.insert("LOG_LEVEL".to_owned(), "debug".to_owned());
        vars.insert("MODE".to_owned(), "normal".to_owned());
        vars
    }

    #[test]
    fn substitute_known_var() {
        // ARRANGE
        let vars = env();

        // ACT
        let result = substitute_vars("--log-level $LOG_LEVEL", &vars);

        // ASSERT
        assert_eq!(result, "--log-level debug");
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
        let result = substitute_vars("/sbin/apid --log-level $LOG_LEVEL --mode $MODE", &vars);

        // ASSERT
        assert_eq!(result, "/sbin/apid --log-level debug --mode normal");
    }

    fn make_svc(name: &str, cmd: &str, deps: &[&str]) -> Service {
        Service {
            name: name.to_owned(),
            command: cmd.to_owned(),
            depends_on: deps.iter().copied().map(str::to_owned).collect(),
            sandbox: None,
        }
    }

    fn caps() -> probe::Capabilities {
        probe::Capabilities::empty()
    }

    #[test]
    fn prepare_applies_var_substitution() {
        // ARRANGE
        let defs = vec![make_svc("apid", "/sbin/apid --log-level $LOG_LEVEL", &[])];
        let vars = env();

        // ACT
        let svcs = prepare(defs, &vars).unwrap();

        // ASSERT
        assert_eq!(
            svcs.first().map(|svc| svc.command.as_str()),
            Some("/sbin/apid --log-level debug")
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
        let content = format!(
            r#"name = "testsvc"
command = "/bin/test"
depends_on = []
{UNSANDBOXED}"#
        );
        std::fs::write(dir.path().join("testsvc.service"), content).unwrap();
        std::fs::write(dir.path().join("ignored.txt"), "noise").unwrap();

        // ACT
        let svcs = scan_services(dir.path(), caps()).unwrap();

        // ASSERT
        assert_eq!(svcs.len(), 1);
        assert_eq!(svcs.first().map(|svc| svc.name.as_str()), Some("testsvc"));
    }

    #[test]
    fn scan_services_skips_malformed_file() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let valid = format!(
            r#"name = "good"
command = "/bin/good"
depends_on = []
{UNSANDBOXED}"#
        );
        std::fs::write(dir.path().join("good.service"), valid).unwrap();
        std::fs::write(dir.path().join("bad.service"), "not valid ][[[").unwrap();

        // ACT
        let svcs = scan_services(dir.path(), caps()).unwrap();

        // ASSERT
        assert_eq!(svcs.len(), 1);
        assert_eq!(svcs.first().map(|svc| svc.name.as_str()), Some("good"));
    }

    #[test]
    fn scan_services_skips_service_without_sandbox_table() {
        // ARRANGE
        let dir = tempfile::tempdir().unwrap();
        let bare = r#"name = "bare"
command = "/bin/bare"
depends_on = []
"#;
        std::fs::write(dir.path().join("bare.service"), bare).unwrap();

        // ACT
        let svcs = scan_services(dir.path(), caps()).unwrap();

        // ASSERT
        assert!(svcs.is_empty(), "an unsandboxed service must be refused");
    }

    #[test]
    fn unknown_capability_name_is_refused() {
        // ARRANGE
        let typo = r#"name = "typo"
command = "/bin/typo"
depends_on = []

[sandbox]
profile = "strict"
capabilities = ["chown", "chownn"]
"#;
        let service = toml::from_str::<Service>(typo).unwrap();
        // What a policy that keeps capabilities requires, so resolution would
        // succeed and only the name itself can fail.
        let caps = probe::Capabilities::CAPSET.insert(probe::Capabilities::CAP_BOUNDING);

        // ACT
        let error = format!("{:#}", validate(&service, caps).unwrap_err());

        // ASSERT
        assert!(
            error.contains("unknown capability: chownn"),
            "expected the offending name in {error:?}"
        );
    }

    fn service_paths(root: &Path) -> Vec<std::path::PathBuf> {
        let dirs = std::fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().path());
        dirs.flat_map(|dir| std::fs::read_dir(dir).unwrap())
            .map(|file| file.unwrap().path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("service"))
            .collect()
    }

    fn assert_sandboxed(path: &Path) {
        let content = std::fs::read_to_string(path).unwrap();
        let service = toml::from_str::<Service>(&content).unwrap();
        let sandbox = service
            .sandbox
            .as_ref()
            .unwrap_or_else(|| panic!("{} has no [sandbox] table", path.display()));
        sandbox
            .policy()
            .unwrap_or_else(|error| panic!("{}: {error:#}", path.display()));
        // An absent table maps to the empty set, so the limits are asserted
        // too: a file that drops its `[sandbox.cgroup]` block must fail here.
        let limits = sandbox
            .cgroup_limits()
            .unwrap_or_else(|error| panic!("{}: {error:#}", path.display()));
        assert_eq!(
            limits,
            cgroup::Limits::new().pids_max(512),
            "{} must keep its shipped cgroup limits",
            path.display()
        );
    }

    #[test]
    fn every_shipped_service_declares_a_sandbox_table() {
        // ARRANGE
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../services");

        // ACT
        let paths = service_paths(&root);
        for path in &paths {
            assert_sandboxed(path);
        }
        assert_sandboxed(&root.join("fleetd/package/fleetd.service"));

        // ASSERT
        assert_eq!(paths.len(), 7, "expected seven base service files");
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
        let result = substitute_vars("héllo $LOG_LEVEL wörld", &vars);

        // ASSERT
        assert_eq!(result, "héllo debug wörld");
    }

    #[test]
    fn substitute_vars_preserves_unknown_var_between_multi_byte_characters() {
        // ARRANGE
        let vars = HashMap::new();

        // ACT
        let result = substitute_vars("héllo $LOG_LEVEL wörld", &vars);

        // ASSERT
        assert_eq!(result, "héllo $LOG_LEVEL wörld");
    }
}
