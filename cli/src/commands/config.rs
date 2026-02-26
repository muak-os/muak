use anyhow::{Context, Result};
use clap::Subcommand;
use tonic::transport::Channel;

use crate::client::{GetConfigRequest, ProvisionServiceClient};
use crate::format::{format_timestamp, time::TimeSeparator};
use crate::ui;

#[derive(Subcommand)]
pub enum ConfigAction {
    Generate,
    Export,
}

/// Handles config subcommands.
pub async fn handle(channel: Channel, action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Generate => {
            unreachable!("Generate is handled in main before connecting")
        }
        ConfigAction::Export => {
            let mut client = ProvisionServiceClient::new(channel);
            let request = tonic::Request::new(GetConfigRequest {});

            let response = client.get_config(request).await?;
            let resp = response.into_inner();

            if !resp.error.is_empty() {
                eprintln!(
                    "{} {}",
                    ui::style::error("Error:"),
                    ui::style::error_text(&resp.error)
                );
                std::process::exit(1);
            }

            let config_str = String::from_utf8(resp.config).context("Invalid UTF-8 in config")?;

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            let timestamp = format_timestamp(now.as_secs() as i64, TimeSeparator::Filename);
            let filename = format!("config-{timestamp}.toml");

            std::fs::write(&filename, &config_str)?;
            println!(
                "{}",
                ui::style::success(&format!("Exported config to {filename}"))
            );
        }
    }

    Ok(())
}
