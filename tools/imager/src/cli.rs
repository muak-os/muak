//! Command-line interface for the imager.

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::profile::Profile;

#[derive(Debug, Parser)]
#[command(name = "imager")]
#[command(about = "Build Muak boot artifacts")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    ProfileId {
        #[arg(short, long)]
        profile: PathBuf,
    },
}

/// Runs the CLI from a caller-provided argument iterator.
///
/// # Errors
///
/// Returns an error when argument parsing fails or when the requested command fails.
#[expect(
    clippy::unused_async,
    reason = "async will be needed when build/resolve commands are added in later phases"
)]
pub async fn run_from<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = Cli::parse_from(args);
    run_command(args.command)
}

/// Like `run_from` but returns an exit code (0 for success, 1 for error).
pub async fn run_with<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    match run_from(args).await {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("Error: {error:?}");
            1
        }
    }
}

/// Runs the CLI from the process's `std::env::args_os`.
#[must_use]
pub async fn run() -> i32 {
    run_with(std::env::args_os()).await
}

fn run_command(command: Command) -> Result<()> {
    match command {
        Command::ProfileId { profile } => {
            let bytes = std::fs::read(&profile)?;
            let spec = Profile::from_toml(&bytes)?;
            let id = spec.profile_id()?;
            println!("{id}");
            Ok(())
        }
    }
}
