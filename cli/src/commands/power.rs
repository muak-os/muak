//! Power management.

use anyhow::{Context as _, Result};
use tonic::transport::Channel;
use tui::prompt::Prompter;
use tui::style::Role;

use crate::client::provision_service::provision_service_client::ProvisionServiceClient;
use crate::client::provision_service::{RebootRequest, ShutdownRequest};
use crate::console::Console;

/// Requests a reboot of the target system.
pub async fn reboot(client: &mut ProvisionServiceClient<Channel>, console: &Console) -> Result<()> {
    let steps = console.steps()?;

    steps.start("Requesting reboot...");

    let request = tonic::Request::new(RebootRequest {});

    match client.reboot(request).await {
        Ok(_) => steps.complete("Reboot scheduled, the system will restart shortly"),
        Err(e) => {
            steps.fail("Reboot request failed");
            let _finish_result = steps.finish();
            return Err(e).context("Failed to send reboot request");
        }
    }

    steps.finish()?;

    Ok(())
}

/// Requests a shutdown of the target system.
pub async fn shutdown(
    client: &mut ProvisionServiceClient<Channel>,
    console: &Console,
) -> Result<()> {
    let confirmed = match Prompter::stdio() {
        Some(mut prompter) => prompter.confirm("Shutdown the system?")?,
        None => false,
    };

    if !confirmed {
        console.line(Role::Warn, "Shutdown cancelled.");
        return Ok(());
    }

    let steps = console.steps()?;

    steps.start("Requesting shutdown...");

    let request = tonic::Request::new(ShutdownRequest {});

    match client.shutdown(request).await {
        Ok(_) => steps.complete("Shutdown scheduled, the system will power off shortly"),
        Err(e) => {
            steps.fail("Shutdown request failed");
            let _finish_result = steps.finish();
            return Err(e).context("Failed to send shutdown request");
        }
    }

    steps.finish()?;

    Ok(())
}
