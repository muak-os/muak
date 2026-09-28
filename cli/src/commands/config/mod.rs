//! System config inspection and generation.

mod audit;

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::Subcommand;
use tonic::transport::Channel;

use crate::client::provision_service::{
    GetConfigRequest, GetConfigSnapshotRequest, GetDefaultConfigRequest,
    provision_service_client::ProvisionServiceClient,
};
use crate::ui;

#[derive(Subcommand, Clone)]
pub enum Action {
    Generate,
    Get {
        #[arg(long)]
        from: Option<String>,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    History {
        #[arg(long, short, default_value = "10")]
        limit: u32,
    },
    Diff {
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
    },
}

/// Handles config subcommands.
pub async fn handle(channel: Channel, action: Action) -> Result<()> {
    match action {
        Action::Generate => generate(channel).await,
        Action::Get { from, output } => get(channel, from, output).await,
        Action::History { limit } => audit::history(channel, limit).await,
        Action::Diff { from, to } => audit::diff(channel, from, to).await,
    }
}

async fn generate(channel: Channel) -> Result<()> {
    let mut client = ProvisionServiceClient::new(channel);
    let resp = client
        .get_default_config(tonic::Request::new(GetDefaultConfigRequest {}))
        .await?
        .into_inner();
    if !resp.error.is_empty() {
        return Err(anyhow::anyhow!("{}", resp.error));
    }

    let config = String::from_utf8(resp.config).context("Invalid UTF-8 in config")?;
    print!("{config}");
    Ok(())
}

async fn get(channel: Channel, from: Option<String>, output: Option<PathBuf>) -> Result<()> {
    let mut client = ProvisionServiceClient::new(channel);
    let config = match from.as_deref() {
        Some(update_id) => fetch_snapshot(&mut client, update_id).await?,
        None => fetch_current(&mut client).await?,
    };

    match output {
        Some(path) => {
            std::fs::write(&path, &config)
                .with_context(|| format!("Failed to write config to {}", path.display()))?;
            println!(
                "{}",
                ui::style::success(&format!("Config written to {}", path.display()))
            );
        }
        None => print!("{config}"),
    }

    Ok(())
}

async fn fetch_current(client: &mut ProvisionServiceClient<Channel>) -> Result<String> {
    let resp = client
        .get_config(tonic::Request::new(GetConfigRequest {}))
        .await?
        .into_inner();
    if !resp.error.is_empty() {
        return Err(anyhow::anyhow!("{}", resp.error));
    }
    String::from_utf8(resp.config).context("Invalid UTF-8 in config")
}

pub(crate) async fn fetch_snapshot(
    client: &mut ProvisionServiceClient<Channel>,
    update_id: &str,
) -> Result<String> {
    let resp = client
        .get_config_snapshot(tonic::Request::new(GetConfigSnapshotRequest {
            update_id: update_id.to_owned(),
        }))
        .await?
        .into_inner();
    if !resp.error.is_empty() {
        return Err(anyhow::anyhow!("{}", resp.error));
    }
    String::from_utf8(resp.config).context("Invalid UTF-8 in config snapshot")
}
