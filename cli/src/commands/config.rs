use anyhow::{Context, Result};
use clap::Subcommand;
use tonic::transport::Channel;

use crate::client::{
    GetConfigHistoryRequest, GetConfigRequest, GetConfigSnapshotRequest, ProvisionServiceClient,
};
use crate::format::{format_timestamp, time::TimeSeparator};
use crate::ui;

#[derive(Subcommand)]
pub enum ConfigAction {
    Generate,
    Export,
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
pub async fn handle(channel: Channel, action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Generate => {
            unreachable!("Generate is handled in main before connecting")
        }
        ConfigAction::Export => export(channel).await,
        ConfigAction::History { limit } => history(channel, limit).await,
        ConfigAction::Diff { from, to } => diff(channel, from, to).await,
    }
}

async fn export(channel: Channel) -> Result<()> {
    let mut client = ProvisionServiceClient::new(channel);
    let response = client
        .get_config(tonic::Request::new(GetConfigRequest {}))
        .await?;
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

    Ok(())
}

async fn history(channel: Channel, limit: u32) -> Result<()> {
    let mut client = ProvisionServiceClient::new(channel);
    let response = client
        .get_config_history(tonic::Request::new(GetConfigHistoryRequest { limit }))
        .await?;
    let resp = response.into_inner();

    if !resp.error.is_empty() {
        eprintln!(
            "{} {}",
            ui::style::error("Error:"),
            ui::style::error_text(&resp.error)
        );
        std::process::exit(1);
    }

    if resp.entries.is_empty() {
        println!("{}", ui::style::muted("No config history found."));
        return Ok(());
    }

    let table = resp.entries.iter().fold(
        ui::Table::new().header(&["TIMESTAMP", "UPDATE ID", "KIND", "AUTHOR"]),
        |t, e| {
            t.row(&[
                &format_timestamp(e.timestamp, TimeSeparator::Display),
                &e.update_id,
                &e.change_kind,
                &e.author,
            ])
        },
    );

    table.print();
    Ok(())
}

async fn diff(channel: Channel, from: Option<String>, to: Option<String>) -> Result<()> {
    let from_update_id = from.unwrap_or_default();
    let to_update_id = to.unwrap_or_default();

    if from_update_id.is_empty() && to_update_id.is_empty() {
        eprintln!(
            "{} {}",
            ui::style::error("Error:"),
            ui::style::error_text("At least one of --from or --to must be specified.")
        );
        std::process::exit(1);
    }

    let mut client = ProvisionServiceClient::new(channel);

    let from = fetch_config_at(&mut client, &from_update_id).await?;
    let to = fetch_config_at(&mut client, &to_update_id).await?;

    let changes = diff_configs(&from, &to)?;

    if changes.is_empty() {
        println!("{}", ui::style::muted("No differences found."));
        return Ok(());
    }

    let table = changes.iter().fold(
        ui::Table::new().header(&["FIELD", "BEFORE", "AFTER"]),
        |t, (field, before, after)| {
            t.row(&[
                field.as_str(),
                &ui::style::negative(before).to_string(),
                &ui::style::positive(after).to_string(),
            ])
        },
    );

    table.print();
    Ok(())
}

async fn fetch_config_at(
    client: &mut ProvisionServiceClient<Channel>,
    update_id: &str,
) -> Result<String> {
    let response = client
        .get_config_snapshot(tonic::Request::new(GetConfigSnapshotRequest {
            update_id: update_id.to_string(),
        }))
        .await?;
    let resp = response.into_inner();
    if !resp.error.is_empty() {
        eprintln!(
            "{} {}",
            ui::style::error("Error:"),
            ui::style::error_text(&resp.error)
        );
        std::process::exit(1);
    }
    String::from_utf8(resp.config).context("Invalid UTF-8 in config snapshot")
}

fn diff_configs(from: &str, to: &str) -> Result<Vec<(String, String, String)>> {
    let from: toml::Value = toml::from_str(from).context("Failed to parse 'from' config")?;
    let to: toml::Value = toml::from_str(to).context("Failed to parse 'to' config")?;

    let mut changes = Vec::new();
    diff_values(&mut changes, "", &from, &to);
    Ok(changes)
}

fn diff_values(
    changes: &mut Vec<(String, String, String)>,
    prefix: &str,
    from: &toml::Value,
    to: &toml::Value,
) {
    match (from, to) {
        (toml::Value::Table(f), toml::Value::Table(t)) => {
            for (key, fval) in f {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", prefix, key)
                };
                match t.get(key) {
                    Some(tval) => diff_values(changes, &path, fval, tval),
                    None => changes.push((path, fval.to_string(), String::new())),
                }
            }
            for (key, tval) in t {
                if !f.contains_key(key) {
                    let path = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{}.{}", prefix, key)
                    };
                    changes.push((path, String::new(), tval.to_string()));
                }
            }
        }
        (f, t) if f != t => changes.push((prefix.to_string(), f.to_string(), t.to_string())),
        _ => {}
    }
}
