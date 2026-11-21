//! Spawning supervised services through the sandbox crate.

use alloc::ffi::CString;
use std::ffi::{CStr, OsStr};
use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStringExt as _;
use std::path::Path;

use anyhow::{Context as _, Error, Result, bail};
use sandbox::cgroup::v2::Tree;
use sandbox::spawn::{self, Cmd};
use sandbox::{cgroup, probe};

use super::service::{ServiceState, ServiceStatus};

/// Descriptor the child expects its prebound listener on.
const LISTENER_FD: RawFd = 3;

/// Output file descriptors produced by spawning a service.
pub(crate) struct SpawnResult {
    pub pid: i32,
    pub stdout: OwnedFd,
    pub stderr: OwnedFd,
}

/// Abstraction over spawning a supervised child process.
pub(crate) trait Spawn {
    fn spawn(&mut self, state: &mut ServiceState) -> Result<SpawnResult>;
}

/// Spawns real OS processes under the policy of the service file.
pub(crate) struct Spawner {
    caps: probe::Capabilities,
}

impl Spawner {
    /// Returns a spawner that resolves policies against `caps`.
    pub(crate) const fn new(caps: probe::Capabilities) -> Self {
        Self { caps }
    }
}

impl Spawn for Spawner {
    fn spawn(&mut self, state: &mut ServiceState) -> Result<SpawnResult> {
        let sandbox = state
            .service
            .sandbox
            .as_ref()
            .context("service has no [sandbox] table")?;
        let name = state.service.name.as_str();
        if !cgroup::valid_name(name) {
            bail!("service name `{name}` is not a valid cgroup name");
        }
        let cgroup_limits = sandbox
            .cgroup_limits()
            .context("invalid [sandbox.cgroup] table")?;
        let policy = sandbox.policy().context("invalid [sandbox] table")?;
        let resolved = policy
            .resolve(&self.caps)
            .context("[sandbox] policy cannot be honoured on this machine")?;

        let tokens = state.service.argv();
        let program = tokens.first().context("command must not be empty")?;
        if !Path::new(program).exists() {
            bail!("Binary not found: {program}");
        }

        let (stdout_reader, stdout_writer) =
            std::io::pipe().context("Failed to create the stdout pipe")?;
        let (stderr_reader, stderr_writer) =
            std::io::pipe().context("Failed to create the stderr pipe")?;
        let listener = state
            .listener_fd
            .as_ref()
            .map(|fd| (fd.as_fd(), LISTENER_FD));
        let pass: &[(BorrowedFd<'_>, RawFd)] = listener.as_slice();

        let tree = Tree::claim(Path::new(crate::cgroup::ROOT), name, &cgroup_limits)
            .with_context(|| format!("Failed to create the cgroup of {name}"))?;

        let child = (|| {
            let (exe, argv) = command_line(&tokens)?;
            let envp = environment()?;
            let argv: Vec<&CStr> = argv.iter().map(CString::as_c_str).collect();
            let envp: Vec<&CStr> = envp.iter().map(CString::as_c_str).collect();
            let cmd = Cmd::new(&exe, &argv, &envp)
                .stdout(stdout_writer.as_fd())
                .stderr(stderr_writer.as_fd())
                .pass(pass);
            let cgroup = tree
                .fd()
                .with_context(|| format!("Failed to open the cgroup of {name}"))?;

            spawn::run(&resolved, &cmd, Some(cgroup)).context("Failed to spawn service")
        })();

        let child = match child {
            Ok(child) => child,
            Err(error) => return Err(discard(tree, name, error)),
        };
        drop(stdout_writer);
        drop(stderr_writer);

        let pid = child.pid().as_raw_nonzero().get();
        state.pid = Some(pid);
        state.status = ServiceStatus::Starting;

        kmsg::info!("Spawned {} with PID {}", state.service.name, pid);

        Ok(SpawnResult {
            pid,
            stdout: OwnedFd::from(stdout_reader),
            stderr: OwnedFd::from(stderr_reader),
        })
    }
}

// Removes the cgroup of a service whose spawn failed and hands the failure back.
fn discard(tree: Tree<'_>, name: &str, error: Error) -> Error {
    if let Err(failed) = tree.remove() {
        kmsg::warn!("cgroup: removing the cgroup of {name} after a failed spawn failed: {failed}");
    }

    error
}

/// Returns the program and the argument vector of `tokens`.
fn command_line(tokens: &[String]) -> Result<(CString, Vec<CString>)> {
    let program = tokens.first().context("command must not be empty")?;
    let program = cstring(program.as_bytes())?;
    let argv = cstrings(&tokens.iter().map(String::as_bytes).collect::<Vec<_>>())?;

    Ok((program, argv))
}

fn cstring(bytes: &[u8]) -> Result<CString> {
    CString::new(bytes).context("an argument contains a NUL byte")
}

fn cstrings(bytes: &[&[u8]]) -> Result<Vec<CString>> {
    bytes.iter().map(|item| cstring(item)).collect()
}

fn environment() -> Result<Vec<CString>> {
    let entries = std::env::vars_os().map(|(key, value)| {
        let mut entry = key;
        entry.push(OsStr::new("="));
        entry.push(&value);
        entry.into_vec()
    });
    let entries: Vec<_> = entries.collect();
    let entries: Vec<&[u8]> = entries.iter().map(Vec::as_slice).collect();

    cstrings(&entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::supervisor::service::Service;

    fn make_state(document: &str) -> ServiceState {
        ServiceState::new(toml::from_str::<Service>(document).expect("service document"))
    }

    fn spawner() -> Spawner {
        Spawner::new(probe::Capabilities::empty())
    }

    #[test]
    fn command_line_keeps_the_program_as_the_first_argument() {
        // ARRANGE
        let tokens = ["/sbin/apid", "--maintenance", "false"].map(str::to_owned);

        // ACT
        let (program, argv) = command_line(&tokens).expect("command line");

        // ASSERT
        assert_eq!(program.to_str(), Ok("/sbin/apid"));
        let argv: Vec<_> = argv
            .iter()
            .map(|item| item.to_str().expect("utf8 argument"))
            .collect();
        assert_eq!(argv, ["/sbin/apid", "--maintenance", "false"]);
    }

    #[test]
    fn empty_command_returns_error() {
        // ARRANGE
        let mut state = make_state(
            "name = \"test-svc\"\ncommand = \"\"\ndepends_on = []\n\n[sandbox]\nprofile = \"none\"\n",
        );

        // ACT
        let result = spawner().spawn(&mut state);

        // ASSERT
        let err = result.err().expect("expected an error");
        assert!(err.to_string().contains("command must not be empty"));
    }

    #[test]
    fn nonexistent_binary_returns_error() {
        // ARRANGE
        let mut state = make_state(
            "name = \"test-svc\"\ncommand = \"/nonexistent/binary/xyz_abc\"\ndepends_on = []\n\n[sandbox]\nprofile = \"none\"\n",
        );

        // ACT
        let result = spawner().spawn(&mut state);

        // ASSERT
        let err = result.err().expect("expected an error");
        assert!(err.to_string().contains("Binary not found"));
    }

    #[test]
    fn missing_sandbox_table_returns_error() {
        // ARRANGE
        let mut state =
            make_state("name = \"test-svc\"\ncommand = \"/bin/true\"\ndepends_on = []\n");

        // ACT
        let result = spawner().spawn(&mut state);

        // ASSERT
        let err = format!("{:#}", result.err().expect("expected an error"));
        assert!(
            err.contains("no [sandbox] table"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn a_name_that_is_not_a_cgroup_name_returns_error() {
        // ARRANGE
        let mut state = make_state(
            "name = \"a/b\"\ncommand = \"/bin/true\"\ndepends_on = []\n\n[sandbox]\nprofile = \"none\"\n",
        );

        // ACT
        let result = spawner().spawn(&mut state);

        // ASSERT
        let err = result.err().expect("expected an error");
        assert!(
            err.to_string().contains("not a valid cgroup name"),
            "unexpected error: {err}"
        );
    }
}
