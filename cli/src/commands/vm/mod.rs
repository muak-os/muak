mod create;
mod lifecycle;
mod list;
mod logs;

use anyhow::Result;
use clap::Subcommand;
use tonic::transport::Channel;

use crate::client::vm_service::vm_service_client::VmServiceClient;

#[derive(Subcommand, Clone)]
pub enum Action {
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
pub async fn handle(client: &mut VmServiceClient<Channel>, action: Action) -> Result<()> {
    match action {
        Action::Create {
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
                client,
                create::VmSpec {
                    name,
                    cmdline,
                    kernel,
                    initrd,
                    vmm,
                    cpus,
                    memory,
                    disk,
                    disk_size,
                },
            )
            .await
        }
        Action::Start { vm_id } => lifecycle::handle_start(client, vm_id).await,
        Action::Stop { vm_id, force } => lifecycle::handle_stop(client, vm_id, force).await,
        Action::Delete { vm_id } => lifecycle::handle_delete(client, vm_id).await,
        Action::Logs { vm_id, tail } => logs::handle(client, vm_id, tail).await,
        Action::List => list::handle(client).await,
    }
}
