//! Config history and diff inspection.

use anyhow::{Context as _, Result};
use tonic::transport::Channel;

use crate::client::provision_service::{
    ConfigHistoryEntry, GetConfigHistoryRequest, provision_service_client::ProvisionServiceClient,
};
use crate::commands::config::fetch_snapshot;
use crate::format::time::format_timestamp;
use crate::ui;

pub(crate) async fn history(channel: Channel, limit: u32) -> Result<()> {
    let mut client = ProvisionServiceClient::new(channel);
    let response = client
        .get_config_history(tonic::Request::new(GetConfigHistoryRequest { limit }))
        .await?;
    let resp = response.into_inner();

    if !resp.error.is_empty() {
        return Err(anyhow::anyhow!("{}", resp.error));
    }

    if resp.entries.is_empty() {
        println!("{}", ui::style::muted("No config history found."));
        return Ok(());
    }

    let table = resp.entries.iter().fold(
        ui::table::Table::new().header(&["TIMESTAMP", "UPDATE ID", "KIND", "AUTHOR"]),
        |table, entry| {
            table.row(&[
                &format_timestamp(entry.timestamp),
                &entry.update_id,
                &entry.change_kind,
                &entry.author,
            ])
        },
    );

    table.print();
    Ok(())
}

pub(crate) async fn diff(channel: Channel, from: Option<String>, to: Option<String>) -> Result<()> {
    let mut client = ProvisionServiceClient::new(channel);

    let (from, to) = match (from, to) {
        (Some(from_id), Some(to_id)) => {
            let before = fetch_snapshot(&mut client, &from_id).await?;
            let after = fetch_snapshot(&mut client, &to_id).await?;
            (before, after)
        }
        (None, Some(to_id)) => {
            let entries = fetch_history(&mut client).await?;
            let predecessor = entries
                .iter()
                .skip_while(|entry| entry.update_id != to_id)
                .nth(1)
                .map(|entry| entry.update_id.as_str());
            if let Some(prev) = predecessor {
                let before = fetch_snapshot(&mut client, prev).await?;
                let after = fetch_snapshot(&mut client, &to_id).await?;
                (before, after)
            } else {
                println!(
                    "{}",
                    ui::style::muted("No previous entry to compare against.")
                );
                return Ok(());
            }
        }
        _ => {
            return Err(anyhow::anyhow!(
                "Specify --to <update-id>, or both --from <update-id> --to <update-id>."
            ));
        }
    };

    let changes = config::system::diff(&from, &to).context("Failed to diff configs")?;

    if changes.is_empty() {
        println!("{}", ui::style::muted("No differences found."));
        return Ok(());
    }

    let mut table = ui::table::Table::new().header(&["FIELD", "BEFORE", "AFTER"]);
    for (field, before, after) in changes {
        table = table.row(&[
            field.as_str(),
            &ui::style::negative(&before).to_string(),
            &ui::style::positive(&after).to_string(),
        ]);
    }

    table.print();
    Ok(())
}

async fn fetch_history(
    client: &mut ProvisionServiceClient<Channel>,
) -> Result<Vec<ConfigHistoryEntry>> {
    let resp = client
        .get_config_history(tonic::Request::new(GetConfigHistoryRequest { limit: 0 }))
        .await?
        .into_inner();
    if !resp.error.is_empty() {
        return Err(anyhow::anyhow!("{}", resp.error));
    }
    Ok(resp.entries)
}
