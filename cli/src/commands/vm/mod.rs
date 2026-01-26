mod create;
mod lifecycle;
mod list;
mod logs;

use anyhow::Result;
use tonic::transport::Channel;

use crate::VmAction;
use crate::client::VmServiceClient;

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
