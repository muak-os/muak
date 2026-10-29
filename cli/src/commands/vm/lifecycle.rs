use anyhow::Result;
use tonic::transport::Channel;

use crate::client::vm_service::vm_service_client::VmServiceClient;
use crate::client::vm_service::{DeleteVmRequest, StartVmRequest, StopVmRequest};
use crate::console::Console;

/// Starts a VM.
pub async fn handle_start(
    client: &mut VmServiceClient<Channel>,
    vm_id: String,
    console: &Console,
) -> Result<()> {
    let request = tonic::Request::new(StartVmRequest {
        vm_id: vm_id.clone(),
    });

    let response = client.start_vm(request).await?;
    let resp = response.into_inner();

    if resp.success {
        console.success(&format!("Started VM: {vm_id}"));
        Ok(())
    } else {
        Err(anyhow::anyhow!("Failed to start VM: {}", resp.error))
    }
}

/// Stops a VM.
pub async fn handle_stop(
    client: &mut VmServiceClient<Channel>,
    vm_id: String,
    force: bool,
    console: &Console,
) -> Result<()> {
    let request = tonic::Request::new(StopVmRequest {
        vm_id: vm_id.clone(),
        force,
    });

    let response = client.stop_vm(request).await?;
    let resp = response.into_inner();

    if resp.success {
        console.success(&format!("Stopped VM: {vm_id}"));
        Ok(())
    } else {
        Err(anyhow::anyhow!("Failed to stop VM: {}", resp.error))
    }
}

/// Deletes a VM.
pub async fn handle_delete(
    client: &mut VmServiceClient<Channel>,
    vm_id: String,
    console: &Console,
) -> Result<()> {
    let request = tonic::Request::new(DeleteVmRequest {
        vm_id: vm_id.clone(),
    });

    let response = client.delete_vm(request).await?;
    let resp = response.into_inner();

    if resp.success {
        console.success(&format!("Deleted VM: {vm_id}"));

        Ok(())
    } else {
        Err(anyhow::anyhow!("Failed to delete VM: {}", resp.error))
    }
}
