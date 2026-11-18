//! Spawning supervised services through the sandbox crate.
//!
//! Every service goes through [`sandbox::spawn::run`], so exactly one spawn
//! path exists and it always applies the policy of the service `[sandbox]`
//! table. The pre-bound listener is installed at fd 3 with `Cmd::pass` instead
//! of a `pre_exec` hook, which keeps the child side inside the sandbox crate
//! where the allocation and lock discipline of the fork window is enforced.
//!
//! # No pidfd wait
//!
//! The `Reaper` already awaits `SIGCHLD` and drains `waitpid(-1, WNOHANG)`, so
//! nothing here ever blocks a tokio worker on a child. `Child::pidfd` stays
//! available for a consumer that needs it later; do not add pidfd machinery
//! here to wait for exits.

use alloc::ffi::CString;
use std::ffi::{CStr, OsStr};
use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStringExt as _;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use sandbox::probe;
use sandbox::spawn::{self, Cmd};

use super::service::{ServiceState, ServiceStatus};

/// Descriptor the child expects its pre-bound listener on.
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
    pub(crate) fn new(caps: probe::Capabilities) -> Self {
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

        // The strings and the command live only across the fork: the sandbox
        // crate turns them into pointers before the child runs, and the child
        // copies nothing.
        let child = {
            let (program, argv) = command_line(&tokens)?;
            let envp = environment()?;
            let argv: Vec<&CStr> = argv.iter().map(CString::as_c_str).collect();
            let envp: Vec<&CStr> = envp.iter().map(CString::as_c_str).collect();
            let cmd = Cmd::new(&program, &argv, &envp)
                .stdout(stdout_writer.as_fd())
                .stderr(stderr_writer.as_fd())
                .pass(pass);
            spawn::run(&resolved, &cmd, None).context("Failed to spawn service")?
        };
        // The parent must not hold the write ends: the log reader would never
        // see the end of the output of a service that exited.
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

/// Returns the program and the argument vector of `tokens`.
///
/// `execve` reads the program name from `argv[0]`, and `Service::argv` keeps
/// the program as its first element, so the vector is built from every token
/// and never from the arguments alone.
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
}
