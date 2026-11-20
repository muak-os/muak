//! Command-line interface for the catalog publisher.

mod compose;
mod publish;
mod remove;
mod report;
mod verify;

use std::ffi::OsString;

use anyhow::Result;
use clap::{Parser, Subcommand};
use kata::progress::Reporter;

// Default registry prefix when `REGISTRY` is unset or empty.
const DEFAULT_REGISTRY: &str = "ghcr.io/muak-os";

#[derive(Parser, Debug)]
#[command(name = env!("CARGO_PKG_NAME"))]
#[command(about = env!("CARGO_PKG_DESCRIPTION"))]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Verify(verify::Args),
    Publish(publish::Args),
    Compose(compose::Args),
    Remove(remove::Args),
}

/// Run the CLI from a caller-provided argument iterator.
///
/// # Errors
///
/// Returns an error if argument parsing fails or the requested operation fails.
pub(crate) fn run_from<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = Args::parse_from(args);
    let reporter = report::Plain;
    run_command(args.command, &reporter)
}

/// Runs the CLI with the given arguments and returns an exit code.
#[must_use = "callers must propagate the process exit code"]
pub(crate) fn run_with<I, T>(args: I) -> i32
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
#[must_use = "callers must propagate the process exit code"]
pub(crate) fn run() -> i32 {
    run_with(std::env::args_os())
}

/// Registry prefix for catalog images: `REGISTRY`, then the default.
#[must_use = "callers need the prefix to build image references"]
pub(crate) fn registry_prefix() -> String {
    std::env::var("REGISTRY")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_REGISTRY.to_owned())
}

/// Parse an architecture name into the architecture recorded in the image config.
pub(crate) fn parse_arch(arch: &str) -> Result<koci::arch::Arch, String> {
    arch.parse()
}

fn run_command(command: Command, reporter: &dyn Reporter) -> Result<()> {
    match command {
        Command::Verify(args) => verify::run(args, reporter),
        Command::Publish(args) => publish::run(args, reporter),
        Command::Compose(args) => compose::run(args, reporter),
        Command::Remove(args) => remove::run(args, reporter),
    }
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
            "publish",
            "--release",
            "v1.2.3",
            "--dir",
            missing.to_str().expect("path must be valid utf-8"),
        ])
        .expect_err("publish should fail for a missing root");

        // ASSERT
        assert!(error.to_string().contains("Failed to publish"));
    }

    #[test]
    fn run_with_returns_non_zero_for_errors() {
        // ARRANGE
        let workspace = TempDir::new().expect("create temp dir");
        let missing = workspace.path().join("missing");
        let dir = missing.to_str().expect("path must be valid utf-8");

        // ACT
        let exit_code = run_with(["kata", "publish", "--release", "v1.2.3", "--dir", dir]);

        // ASSERT
        assert_eq!(exit_code, 1_i32);
    }
}
