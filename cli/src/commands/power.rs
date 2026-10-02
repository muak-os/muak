//! Power management.

use anyhow::{Context as _, Result};
use tonic::transport::Channel;

use crate::client::provision_service::{
    RebootRequest, ShutdownRequest, provision_service_client::ProvisionServiceClient,
};
use crate::ui;

/// Requests a reboot of the target system.
pub async fn reboot(client: &mut ProvisionServiceClient<Channel>) -> Result<()> {
    let steps = ui::steps::Steps::new();

    steps.start("Requesting reboot...");

    let request = tonic::Request::new(RebootRequest {});

    match client.reboot(request).await {
        Ok(_) => steps.complete("Reboot scheduled, the system will restart shortly"),
        Err(e) => {
            steps.fail("Reboot request failed");
            steps.finish().await;
            return Err(e).context("Failed to send reboot request");
        }
    }

    steps.finish().await;

    Ok(())
}

/// Requests a shutdown of the target system.
pub async fn shutdown(client: &mut ProvisionServiceClient<Channel>) -> Result<()> {
    if !ui::prompt::confirm("Shutdown the system?")? {
        println!("{}", ui::style::warn("Shutdown cancelled."));
        return Ok(());
    }

    let steps = ui::steps::Steps::new();

    steps.start("Requesting shutdown...");

    let request = tonic::Request::new(ShutdownRequest {});

    match client.shutdown(request).await {
        Ok(_) => steps.complete("Shutdown scheduled, the system will power off shortly"),
        Err(e) => {
            steps.fail("Shutdown request failed");
            steps.finish().await;
            return Err(e).context("Failed to send shutdown request");
        }
    }

    steps.finish().await;

    Ok(())
}
