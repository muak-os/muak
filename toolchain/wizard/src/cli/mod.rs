//! Command-line interface for the wizard.

mod build;
mod console;
mod tracked;
mod writer;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};
use console::Console;
use koci::arch::Arch;
use tui::output::Flags;
use wizard::domain::profile::Profile;
use wizard::request::Request;
use wizard::resolver::{self, discover};

#[derive(Debug, Parser)]
#[command(name = "wizard")]
#[command(about = "Build Muak boot artifacts")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    /// Suppress progress and decorative success lines.
    #[arg(long, global = true, conflicts_with = "verbose")]
    quiet: bool,

    /// Disable color while keeping the detected mode.
    #[arg(long, global = true)]
    no_color: bool,

    /// Show per-node and per-item detail.
    #[arg(long, global = true, conflicts_with = "quiet")]
    verbose: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    ProfileId {
        #[arg(short, long)]
        profile: PathBuf,
    },
    Resolve {
        #[arg(short, long)]
        profile: PathBuf,

        #[arg(long)]
        version: String,

        #[arg(long)]
        registry: String,

        #[arg(long, value_parser = wizard::arch::parse)]
        arch: Arch,
    },
    Build(build::Args),
}

/// Runs the CLI with the given arguments.
///
/// # Errors
///
/// Returns an error when argument parsing or command execution fails.
pub fn run_from<I, T>(args: I) -> Result<()>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let Cli {
        quiet,
        no_color,
        verbose,
        command,
    } = Cli::parse_from(args);
    let flags = Flags {
        quiet,
        no_color,
        verbose,
    };

    run_command(command, &Console::detect(flags))
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

fn run_command(command: Command, console: &Console) -> Result<()> {
    match command {
        Command::ProfileId { profile } => run_profile_id(&profile),
        Command::Resolve {
            profile,
            version,
            registry,
            arch,
        } => run_resolve(&profile, &version, &registry, arch),
        Command::Build(args) => build::run(&args, console),
    }
}

fn run_profile_id(profile_path: &Path) -> Result<()> {
    let bytes = std::fs::read(profile_path)?;
    let spec = Profile::from_toml(&bytes)?;
    println!("{}", spec.profile_id()?);

    Ok(())
}

fn run_resolve(profile_path: &Path, version: &str, registry: &str, arch: Arch) -> Result<()> {
    let bytes = std::fs::read(profile_path)
        .with_context(|| format!("read profile {}", profile_path.display()))?;
    let profile = Profile::from_toml(&bytes)?;
    let request = Request::new(version, registry).arch(arch);
    let mut resolved = resolver::plan(&request, &profile)?;
    discover::layout(&mut resolved)?;

    println!("profile id: {}", resolved.profile_id());
    println!("resolution id: {}", resolved.resolution_id());
    println!("disk layout: {}", resolved.build().layout().name());
    println!("resolved installer: {}", resolved.build().installer());
    println!(
        "resolved kernel: {} -> {}",
        resolved.build().kernel().image(),
        resolved.build().kernel().source()
    );
    for ext in resolved.build().extensions() {
        println!(" resolved extension: {} -> {}", ext.name(), ext.source());
    }
    if let Some(ov) = resolved.build().overlay() {
        println!(
            "  resolved overlay: {}/{} -> {}",
            ov.image(),
            ov.name(),
            ov.source_ref()
        );
    }

    Ok(())
}
