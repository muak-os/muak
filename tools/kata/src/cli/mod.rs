//! Command-line interface for the catalog publisher.

use std::ffi::OsString;

use anyhow::Result;
use clap::{Parser, Subcommand};

mod add;
mod init;
mod publish;
mod verify;

const DEFAULT_REGISTRY: &str = "ghcr.io/muak-os";

#[derive(Parser, Debug)]
#[command(name = env!("CARGO_PKG_NAME"))]
#[command(about = env!("CARGO_PKG_DESCRIPTION"))]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Init(init::Args),
    Add(add::Args),
    Verify(verify::Args),
    Publish(publish::Args),
}

/// Run the CLI from a caller-provided argument iterator.
///
/// # Errors
///
/// Returns an error if argument parsing fails or the requested operation fails.
pub fn run_from<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = Args::parse_from(args);
    run_command(args.command)
}

/// Runs the CLI with the given arguments and returns an exit code.
#[must_use]
pub fn run_with<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    match run_from(args) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("Error: {error:?}");
            1
        }
    }
}

/// Runs the CLI with `std::env::args_os()` and returns an exit code.
#[must_use]
pub fn run() -> i32 {
    run_with(std::env::args_os())
}

fn run_command(command: Command) -> Result<()> {
    match command {
        Command::Init(args) => init::run(args),
        Command::Add(args) => add::run(args),
        Command::Verify(args) => verify::run(args),
        Command::Publish(args) => publish::run(args),
    }
}

/// Resolve the registry prefix: explicit flag, then `REGISTRY`, then default.
#[must_use]
pub(crate) fn registry_prefix(explicit: Option<&str>) -> String {
    explicit
        .map(str::to_owned)
        .or_else(|| {
            std::env::var("REGISTRY")
                .ok()
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| DEFAULT_REGISTRY.to_owned())
}

/// Parse an architecture CLI value.
pub(crate) fn parse_arch(arch: &str) -> Result<koci::arch::Arch, String> {
    arch.parse()
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{run_from, run_with};

    #[test]
    fn run_from_reports_unusable_root() {
        // ARRANGE
        let workspace = TempDir::new().expect("create temp dir");
        let missing = workspace.path().join("missing");

        // ACT
        let error = run_from([
            "kata",
            "init",
            "--release",
            "v1.2.3",
            "--dir",
            missing.to_str().expect("path must be valid utf-8"),
        ])
        .expect_err("init should fail for a missing root");

        // ASSERT
        assert!(error.to_string().contains("Failed to seed release"));
    }

    #[test]
    fn run_with_returns_non_zero_for_errors() {
        // ARRANGE
        let workspace = TempDir::new().expect("create temp dir");
        let missing = workspace.path().join("missing");
        let dir = missing.to_str().expect("path must be valid utf-8");

        // ACT
        let exit_code = run_with(["kata", "init", "--release", "v1.2.3", "--dir", dir]);

        // ASSERT
        assert_eq!(exit_code, 1);
    }
}
