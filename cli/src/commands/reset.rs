use anyhow::{Context, Result};
use config::ClientConfig;
use tonic::transport::Channel;

use crate::client::{FactoryResetRequest, ProvisionServiceClient};
use crate::ui;

const CONFIRM_PHRASE: &str = "FACTORY RESET";

/// Handles the reset command.
pub async fn handle(
    client: &mut ProvisionServiceClient<Channel>,
    force: bool,
    context_name: Option<String>,
) -> Result<()> {
    println!(
        "{}",
        ui::style::error("WARNING: This will perform a factory reset!")
    );
    println!(
        "{}",
        ui::style::error_text("All data, VMs, and configuration will be permanently deleted.")
    );
    println!(
        "{}",
        ui::style::error_text("The system will reboot into maintenance mode.\n")
    );

    if !force && !ui::prompt::confirm_phrase("", CONFIRM_PHRASE)? {
        println!("{}", ui::style::warn("Factory reset cancelled."));
        return Ok(());
    }

    let steps = ui::Steps::new();

    steps.start("Initiating factory reset...");

    let request = tonic::Request::new(FactoryResetRequest {});

    let response = match client.factory_reset(request).await {
        Ok(r) => r,
        Err(e) => {
            steps.fail("Factory reset failed");
            steps.finish().await;
            return Err(e).context("Failed to send factory reset request");
        }
    };

    let resp = response.into_inner();

    if !resp.success {
        let msg = format!("Factory reset failed: {}", resp.error);
        steps.fail(&msg);
        steps.finish().await;
        return Err(anyhow::anyhow!("{msg}"));
    }

    steps.complete("Factory reset initiated");

    if let Some(ctx_name) = context_name
        && let Ok(mut config) = ClientConfig::load()
        && config.remove_context(&ctx_name).is_ok()
        && config.save().is_ok()
    {
        let msg = format!("Removed context '{ctx_name}'");
        steps.complete(&msg);
    }

    steps.start("Rebooting into maintenance mode...");
    steps.finish().await;

    Ok(())
}
