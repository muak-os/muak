//! Command-line interface for koci.

use std::ffi::OsString;
use std::path::Path;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use oci::arch::Arch;

mod annotate;
mod copy;
mod merge;
mod pull;
mod push;
mod sign;

/// Top-level CLI arguments.
#[derive(Parser, Debug)]
#[command(name = env!("CARGO_PKG_NAME"))]
#[command(about = env!("CARGO_PKG_DESCRIPTION"))]
struct Args {
    #[command(subcommand)]
    command: Command,
}

/// Available subcommands.
#[derive(Subcommand, Debug)]
enum Command {
    Pull(pull::Args),
    Sign(sign::Args),
    Annotate(annotate::Args),
    Merge(merge::Args),
    Push(push::Args),
    Copy(copy::Args),
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
        Command::Pull(args) => pull::run(args),
        Command::Sign(args) => sign::run(args),
        Command::Annotate(args) => annotate::run(args),
        Command::Merge(args) => merge::run(args),
        Command::Push(args) => push::run(args),
        Command::Copy(args) => copy::run(args),
    }
}

/// Parse an architecture CLI value.
pub(crate) fn parse_arch(arch: &str) -> Result<Arch, String> {
    arch.parse()
}

/// Read a PEM key file.
pub(crate) fn read_key_file(path: &Path) -> Result<String> {
    std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read key from {}", path.display()))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{read_key_file, run_from, run_with};

    #[test]
    fn read_key_file_reports_missing_path() {
        // ARRANGE
        let workspace = TempDir::new().expect("create temp dir");
        let missing = workspace.path().join("missing.pem");

        // ACT
        let error = read_key_file(&missing).expect_err("read should fail");

        // ASSERT
        assert!(error.to_string().contains("Failed to read key from"));
    }

    #[test]
    fn run_from_reports_missing_sign_key() {
        // ARRANGE
        let workspace = TempDir::new().expect("create temp dir");
        let missing = workspace.path().join("missing.pem");

        // ACT
        let error = run_from([
            "koci",
            "sign",
            "--image",
            "repo:test",
            "--key",
            missing.to_str().expect("missing path must be valid utf-8"),
            "--annotation",
            "dev.muak.sig",
        ])
        .expect_err("run_from should fail");

        // ASSERT
        assert!(error.to_string().contains("Failed to read key from"));
    }

    #[test]
    fn run_with_returns_non_zero_for_errors() {
        // ARRANGE
        let workspace = TempDir::new().expect("create temp dir");
        let missing = workspace.path().join("missing.pem");

        // ACT
        let exit_code = run_with([
            "koci",
            "sign",
            "--image",
            "repo:test",
            "--key",
            missing.to_str().expect("missing path must be valid utf-8"),
            "--annotation",
            "dev.muak.sig",
        ]);

        // ASSERT
        assert_eq!(exit_code, 1);
    }
}
