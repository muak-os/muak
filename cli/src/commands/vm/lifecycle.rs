use anyhow::Result;
use owo_colors::OwoColorize;
use tonic::transport::Channel;

use crate::client::{DeleteVmRequest, StartVmRequest, StopVmRequest, VmServiceClient};

/// Starts a VM.
pub async fn handle_start(client: &mut VmServiceClient<Channel>, vm_id: String) -> Result<()> {
    let request = tonic::Request::new(StartVmRequest {
        vm_id: vm_id.clone(),
    });

    let response = client.start_vm(request).await?;
    let resp = response.into_inner();

    if resp.success {
        println!("{}", format!("Started VM: {vm_id}").green());
    } else {
        eprintln!("{}", format!("Error starting VM: {}", resp.error).red());
        std::process::exit(1);
    }

    Ok(())
}

/// Stops a VM.
pub async fn handle_stop(
    client: &mut VmServiceClient<Channel>,
    vm_id: String,
    force: bool,
) -> Result<()> {
    let request = tonic::Request::new(StopVmRequest {
        vm_id: vm_id.clone(),
        force,
    });

    let response = client.stop_vm(request).await?;
    let resp = response.into_inner();

    if resp.success {
        println!("{}", format!("Stopped VM: {vm_id}").green());
    } else {
        eprintln!("{}", format!("Error stopping VM: {}", resp.error).red());
        std::process::exit(1);
    }

    Ok(())
}

/// Deletes a VM.
pub async fn handle_delete(client: &mut VmServiceClient<Channel>, vm_id: String) -> Result<()> {
    let request = tonic::Request::new(DeleteVmRequest {
        vm_id: vm_id.clone(),
    });

    let response = client.delete_vm(request).await?;
    let resp = response.into_inner();

    if resp.success {
        println!("{}", format!("Deleted VM: {vm_id}").green());
    } else {
        eprintln!("{}", format!("Error deleting VM: {}", resp.error).red());
        std::process::exit(1);
    }

    Ok(())
}
