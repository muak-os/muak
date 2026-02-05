use std::io::{Write, stdin, stdout};

use anyhow::{Context, Result, bail};
use owo_colors::OwoColorize;
use tonic::transport::Channel;

use crate::client::{FactoryResetRequest, ProvisionServiceClient};
use crate::config::ClientConfig;

const CONFIRM_PHRASE: &str = "FACTORY RESET";

/// Handles the reset command.
pub async fn handle(
    client: &mut ProvisionServiceClient<Channel>,
    force: bool,
    context_name: Option<String>,
) -> Result<()> {
    println!(
        "{}",
        "WARNING: This will perform a factory reset!".red().bold()
    );
    println!(
        "{}",
        "All data, VMs, and configuration will be permanently deleted.".red()
    );
    println!(
        "{}",
        "The system will reboot into maintenance mode.\n".red()
    );

    if !force && !prompt_confirmation()? {
        println!("{}", "Factory reset cancelled.".yellow());
        return Ok(());
    }

    println!("{}", "Initiating factory reset...".blue());

    let request = tonic::Request::new(FactoryResetRequest {});

    let response = client
        .factory_reset(request)
        .await
        .context("Failed to send factory reset request")?;

    let resp = response.into_inner();

    if resp.success {
        println!("{}", "Factory reset initiated successfully.".green());
        println!("{}", "System will reboot into maintenance mode...".yellow());

        if let Some(ctx_name) = context_name
            && let Ok(mut config) = ClientConfig::load()
            && config.remove_context(&ctx_name).is_ok()
            && config.save().is_ok()
        {
            println!(
                "{}",
                format!("Removed context '{}' (credentials invalidated).", ctx_name).blue()
            );
        }
    } else {
        bail!("Factory reset failed: {}", resp.error);
    }

    Ok(())
}

fn prompt_confirmation() -> Result<bool> {
    print!("Type '{}' to confirm: ", CONFIRM_PHRASE.bold());
    stdout().flush()?;

    let mut input = String::new();
    stdin().read_line(&mut input)?;

    Ok(input.trim() == CONFIRM_PHRASE)
}
