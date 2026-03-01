//! Muak CLI - Command-line interface for managing Muak Linux systems.

mod client;
mod commands;
mod config;
mod format;
pub mod ui;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use commands::auth::AuthAction;
use commands::config::ConfigAction;
use commands::context::ContextAction;
use commands::process::ProcessAction;
use commands::security::SecurityAction;
use commands::vm::VmAction;

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

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
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
        image: Option<String>,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    Reset {
        #[arg(long)]
        force: bool,
    },
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
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = commands::run(cli).await {
        handle_error(&e);
        std::process::exit(1);
    }
}

/// Handles and displays errors with user-friendly messages.
fn handle_error(err: &anyhow::Error) {
    if let Some(status) = err.downcast_ref::<tonic::Status>() {
        let msg = match status.code() {
            tonic::Code::FailedPrecondition if status.message() == "Server not installed" => {
                "Server not installed. Run 'muakctl install --config <config.toml>' to set up."
            }
            tonic::Code::Unauthenticated => {
                "Authentication required. Run 'muakctl auth' to access this resource on the server."
            }
            tonic::Code::PermissionDenied => {
                "Permission denied. You don't have access to this resource."
            }
            tonic::Code::Unavailable => "Server unavailable. Check if the server is running.",
            tonic::Code::DeadlineExceeded => "Request timed out.",
            tonic::Code::NotFound => status.message(),
            tonic::Code::InvalidArgument => status.message(),
            _ => status.message(),
        };
        eprintln!(
            "{} {}",
            ui::style::error("Error:"),
            ui::style::error_text(msg)
        );
    } else {
        let msg = err.to_string();
        eprintln!(
            "{} {}",
            ui::style::error("Error:"),
            ui::style::error_text(&msg)
        );
    }
}
