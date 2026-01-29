mod create;
mod lifecycle;
mod list;
mod logs;

use anyhow::Result;
use clap::Subcommand;
use tonic::transport::Channel;

use crate::client::VmServiceClient;

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

/// Routes VM subcommands to their handlers.
pub async fn handle(client: &mut VmServiceClient<Channel>, action: VmAction) -> Result<()> {
    match action {
        VmAction::Create {
            name,
            cmdline,
            kernel,
            initrd,
            vmm,
            cpus,
            memory,
            disk,
            disk_size,
        } => {
            create::handle(
                client, name, cmdline, kernel, initrd, vmm, cpus, memory, disk, disk_size,
            )
            .await
        }
        VmAction::Start { vm_id } => lifecycle::handle_start(client, vm_id).await,
        VmAction::Stop { vm_id, force } => lifecycle::handle_stop(client, vm_id, force).await,
        VmAction::Delete { vm_id } => lifecycle::handle_delete(client, vm_id).await,
        VmAction::Logs { vm_id, tail } => logs::handle(client, vm_id, tail).await,
        VmAction::List => list::handle(client).await,
    }
}
