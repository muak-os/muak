//! Verification that the freshly installed system comes back online.

use core::time::Duration;
use std::time::Instant;

use anyhow::Result;
use config::user::ServerContext;
use tokio::time::sleep;
use tui::live::Steps;

use crate::client::connect;
use crate::client::provision_service::GetConfigRequest;
use crate::client::provision_service::provision_service_client::ProvisionServiceClient;

/// Polls the server after reboot to verify the install succeeded.
pub(crate) async fn wait(ctx: &ServerContext, steps: &Steps) -> Result<()> {
    steps.start("Rebooting system...");
    steps.start("Waiting for system to come back online...");

    let timeout = Duration::from_mins(5);
    let poll_interval = Duration::from_secs(2);
    let start = Instant::now();

    loop {
        if start.elapsed() > timeout {
            steps.fail("Timed out waiting for system to come back online");
            return Err(anyhow::anyhow!(
                "The installation completed but the system did not respond within 5 minutes.\n\
                 Please check the system manually to verify it booted correctly."
            ));
        }

        let Ok(channel) = connect(ctx, 10).await else {
            sleep(poll_interval).await;
            continue;
        };

        let mut client = ProvisionServiceClient::new(channel);
        match client
            .get_config(tonic::Request::new(GetConfigRequest {}))
            .await
        {
            Ok(_) => {
                steps.complete("System is back online. Installation verified successfully!");
                return Ok(());
            }
            Err(_) => {
                sleep(poll_interval).await;
            }
        }
    }
}
