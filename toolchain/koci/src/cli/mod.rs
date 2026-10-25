//! Command-line interface for koci.

use std::ffi::OsString;
use std::io;
use std::path::Path;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use oci::arch::Arch;
use tui::output::Flags;
use tui::style;

mod console;
mod copy;
mod digest;
mod merge;
mod pull;
mod push;
mod repack;
mod sign;

use console::Console;

// Top-level CLI arguments.
#[derive(Parser, Debug)]
#[command(name = env!("CARGO_PKG_NAME"))]
#[command(about = env!("CARGO_PKG_DESCRIPTION"))]
struct Args {
    /// Suppress progress and decorative success lines.
    #[arg(long, global = true, conflicts_with = "verbose")]
    quiet: bool,

    /// Disable color while keeping the detected mode.
    #[arg(long, global = true)]
    no_color: bool,

    /// Show per-layer and per-blob detail.
    #[arg(long, global = true, conflicts_with = "quiet")]
    verbose: bool,

    #[command(subcommand)]
    command: Command,
}

// Available subcommands.
#[derive(Subcommand, Debug)]
enum Command {
    Pull(pull::Args),
    Sign(sign::Args),
    Merge(merge::Args),
    Push(push::Args),
    Copy(copy::Args),
    Digest(digest::Args),
    Repack(repack::Args),
}

/// Runs the CLI with `std::env::args_os()` and returns an exit code.
#[must_use]
pub fn run() -> i32 {
    run_with(std::env::args_os())
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
            if error
                .chain()
                .filter_map(|cause| cause.downcast_ref::<io::Error>())
                .any(|cause| cause.kind() == io::ErrorKind::BrokenPipe)
            {
                return 0;
            }

            eprintln!("{} {error:?}", style::stderr().error("Error:"));

            1
        }
    }
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
    let Args {
        quiet,
        no_color,
        verbose,
        command,
    } = Args::parse_from(args);
    let flags = Flags {
        quiet,
        no_color,
        verbose,
    };

    run_command(command, &Console::detect(flags))
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

fn run_command(command: Command, console: &Console) -> Result<()> {
    match command {
        Command::Pull(args) => pull::run(args, console),
        Command::Sign(args) => sign::run(args, console),
        Command::Merge(args) => merge::run(args, console),
        Command::Push(args) => push::run(args, console),
        Command::Copy(args) => copy::run(args, console),
        Command::Digest(args) => digest::run(args),
        Command::Repack(args) => repack::run(args, console),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;
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
    fn global_output_flags_parse_before_or_after_the_subcommand() {
        // ARRANGE
        let args = crate::cli::Args::try_parse_from([
            "koci",
            "digest",
            "--image",
            "repo",
            "--tag",
            "v1",
            "--no-color",
            "--verbose",
        ])
        .expect("parse global output flags");

        // ACT
        let quiet = args.quiet;
        let no_color = args.no_color;
        let verbose = args.verbose;

        // ASSERT
        assert!(!quiet);
        assert!(no_color);
        assert!(verbose);
    }

    #[test]
    fn quiet_conflicts_with_verbose() {
        // ARRANGE / ACT
        let error = crate::cli::Args::try_parse_from([
            "koci",
            "--quiet",
            "--verbose",
            "digest",
            "--image",
            "repo",
            "--tag",
            "v1",
        ])
        .expect_err("quiet and verbose must not be combined");

        // ASSERT
        assert!(error.to_string().contains("--verbose"));
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
