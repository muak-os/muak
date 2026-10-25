use anyhow::{Context as _, Result};
use config::user::ClientConfig;
use tonic::transport::Channel;
use tui::prompt::Prompter;
use tui::style::Role;

use crate::client::provision_service::{
    FactoryResetRequest, provision_service_client::ProvisionServiceClient,
};
use crate::console::Console;

const CONFIRM_PHRASE: &str = "FACTORY RESET";

/// Handles the reset command.
pub async fn handle(
    client: &mut ProvisionServiceClient<Channel>,
    force: bool,
    context_name: Option<String>,
    console: &Console,
) -> Result<()> {
    console.line(Role::Error, "WARNING: This will perform a factory reset!");
    console.line(
        Role::ErrorText,
        "All data, VMs, and configuration will be permanently deleted.",
    );
    console.line(
        Role::ErrorText,
        "The system will reboot into maintenance mode.\n",
    );

    if !force {
        let confirmed = match Prompter::stdio() {
            Some(mut prompter) => prompter.confirm_phrase("", CONFIRM_PHRASE)?,
            None => false,
        };

        if !confirmed {
            console.line(Role::Warn, "Factory reset cancelled.");
            return Ok(());
        }
    }

    let steps = console.steps()?;

    steps.start("Initiating factory reset...");

    let request = tonic::Request::new(FactoryResetRequest {});

    let response = match client.factory_reset(request).await {
        Ok(resp) => resp,
        Err(e) => {
            steps.fail("Factory reset failed");
            let _finish_result = steps.finish();
            return Err(e).context("Failed to send factory reset request");
        }
    };

    let resp = response.into_inner();

    if !resp.success {
        let msg = format!("Factory reset failed: {}", resp.error);
        steps.fail(&msg);
        let _finish_result = steps.finish();
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
    steps.finish()?;

    Ok(())
}
