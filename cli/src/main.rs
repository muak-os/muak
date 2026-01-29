mod client;
mod commands;
mod format;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use owo_colors::OwoColorize;

#[derive(Parser)]
#[command(name = "muak")]
#[command(about = env!("CARGO_PKG_DESCRIPTION"), long_about = None)]
pub struct Cli {
    #[arg(long, short, global = true, default_value = "localhost:50051")]
    pub server: String,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
    Process {
        #[command(subcommand)]
        action: ProcessAction,
    },
    Vm {
        #[command(subcommand)]
        action: VmAction,
    },
    Config {
        #[command(subcommand)]
        action: ConfigAction,
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
    },
    Disks,
    Logs,
}

#[derive(Subcommand)]
pub enum AuthAction {
    Requests,
    Approve {
        fingerprint: String,
        #[arg(long, default_value = "read_only")]
        permissions: String,
    },
    Revoke {
        fingerprint: String,
    },
    List,
}

#[derive(Subcommand)]
pub enum ConfigAction {
    Generate,
    Export,
}

#[derive(Subcommand)]
pub enum ProcessAction {
    List,
}

#[derive(Subcommand)]
pub enum VmAction {
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        cmdline: Option<String>,
        #[arg(long)]
        kernel: Option<String>,
        #[arg(long)]
        initrd: Option<String>,
        vmm: String,
        #[arg(long, default_value = "1")]
        cpus: u32,
        #[arg(long, default_value = "512")]
        memory: u64,
        #[arg(long)]
        disk: Vec<String>,
        #[arg(long, default_value = "1024")]
        disk_size: u64,
    },
    Start {
        vm_id: String,
    },
    Stop {
        vm_id: String,
        #[arg(long)]
        force: bool,
    },
    Delete {
        vm_id: String,
    },
    Logs {
        vm_id: String,
        #[arg(long, short = 'n', default_value = "0")]
        tail: i64,
    },
    List,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = commands::run(cli).await {
        handle_error(&e);
        std::process::exit(1);
    }
}

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
        eprintln!("{} {}", "Error:".red().bold(), msg.red());
    } else {
        eprintln!("{} {}", "Error:".red().bold(), err.red());
    }
}
