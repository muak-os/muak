mod client;
mod commands;
mod format;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

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
async fn main() -> Result<()> {
    let cli = Cli::parse();
    commands::run(cli).await
}
