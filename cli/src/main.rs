//! Muak CLI - Command-line interface for managing Muak Linux systems.

extern crate alloc;

mod client;
mod commands;
mod console;
mod format;

use std::io;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use commands::auth::Action as AuthAction;
use commands::config::Action as ConfigAction;
use commands::context::Action as ContextAction;
use commands::process::Action as ProcessAction;
use commands::rollback::Action as RollbackAction;
use commands::security::Action as SecurityAction;
use commands::vm::Action as VmAction;
use console::Console;

#[derive(Parser)]
#[command(name = "muak")]
#[command(about = env!("CARGO_PKG_DESCRIPTION"), long_about = None)]
pub struct Cli {
    #[arg(long, short, global = true)]
    pub endpoint: Option<String>,

    #[arg(long, short = 'c', global = true, env = "MUAK_CONTEXT")]
    pub context: Option<String>,

    #[arg(long, global = true)]
    pub insecure: bool,

    #[arg(long, global = true)]
    pub skip_version_check: bool,

    #[command(flatten)]
    pub output: OutputFlags,

    #[command(subcommand)]
    pub command: Commands,
}

/// Global output flags shared by every subcommand.
#[derive(clap::Args, Clone, Copy, Debug)]
pub struct OutputFlags {
    /// Suppress progress and decorative success lines.
    #[arg(long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Disable color while keeping the detected mode.
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Show per-item and per-upload detail.
    #[arg(long, global = true, conflicts_with = "quiet")]
    pub verbose: bool,
}

#[derive(Subcommand, Clone)]
pub enum Commands {
    Auth {
        #[command(subcommand)]
        action: Option<AuthAction>,
    },
    Process {
        #[command(subcommand)]
        action: ProcessAction,
    },
    Security {
        #[command(subcommand)]
        action: SecurityAction,
    },
    Vm {
        #[command(subcommand)]
        action: VmAction,
    },
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    Rollback {
        #[command(subcommand)]
        action: RollbackAction,
    },
    Context {
        #[command(subcommand)]
        action: ContextAction,
    },
    Install {
        #[arg(long)]
        force: bool,
        #[arg(long)]
        config: PathBuf,
    },
    Update {
        #[arg(long)]
        version: Option<String>,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    Reset {
        #[arg(long)]
        force: bool,
    },
    Reboot,
    Shutdown,
    Disks,
    Dmesg {
        #[arg(long, short)]
        follow: bool,
    },
    Logs {
        #[arg(long, short)]
        service: Option<String>,
        #[arg(long, short)]
        tail: Option<u32>,
        #[arg(long, short)]
        follow: bool,
        #[arg(long, short, value_parser = commands::logs::parse_level)]
        level: Option<i32>,
    },
    Version,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let console = Console::detect(cli.output);
    let Err(error) = commands::run(cli, &console).await else {
        return;
    };

    if error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<io::Error>())
        .any(|cause| cause.kind() == io::ErrorKind::BrokenPipe)
    {
        return;
    }

    handle_error(&error, &console);
    std::process::exit(1);
}

// Handles and displays errors with user-friendly messages.
fn handle_error(error: &anyhow::Error, console: &Console) {
    match error.downcast_ref::<tonic::Status>() {
        Some(status) => console.error(status_message(status)),
        None => console.error(&format!("{error:#}")),
    }
}

// Maps a gRPC status to a user-facing message.
fn status_message(status: &tonic::Status) -> &str {
    match status.code() {
        tonic::Code::FailedPrecondition if status.message() == "Server not installed" => {
            "Server not installed. Run 'muakctl install --config <path>' to set up."
        }
        tonic::Code::Unauthenticated => {
            "Authentication required. Run 'muakctl auth' to access this resource on the server."
        }
        tonic::Code::PermissionDenied => {
            "Permission denied. You don't have access to this resource."
        }
        tonic::Code::Unavailable => "Server unavailable. Check if the server is running.",
        tonic::Code::DeadlineExceeded => "Request timed out.",
        tonic::Code::Ok
        | tonic::Code::Cancelled
        | tonic::Code::Unknown
        | tonic::Code::InvalidArgument
        | tonic::Code::NotFound
        | tonic::Code::AlreadyExists
        | tonic::Code::ResourceExhausted
        | tonic::Code::FailedPrecondition
        | tonic::Code::Aborted
        | tonic::Code::OutOfRange
        | tonic::Code::Unimplemented
        | tonic::Code::Internal
        | tonic::Code::DataLoss => status.message(),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::Cli;

    #[test]
    fn global_output_flags_parse_after_the_subcommand() {
        // ARRANGE
        let cli = Cli::try_parse_from(["muakctl", "version", "--no-color", "--verbose"])
            .expect("parse global output flags");

        // ACT
        let quiet = cli.output.quiet;
        let no_color = cli.output.no_color;
        let verbose = cli.output.verbose;

        // ASSERT
        assert!(!quiet);
        assert!(no_color);
        assert!(verbose);
    }

    #[test]
    fn quiet_conflicts_with_verbose() {
        // ARRANGE / ACT
        let error = Cli::try_parse_from(["muakctl", "--quiet", "--verbose", "version"])
            .err()
            .expect("quiet and verbose must not be combined");

        // ASSERT
        assert!(error.to_string().contains("--verbose"));
    }
}
