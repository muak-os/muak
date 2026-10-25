use anyhow::Result;
use clap::Subcommand;
use tonic::transport::Channel;
use tui::style::Role;
use tui::table::Table;

use crate::client::provision_service::{
    GetRollbackHistoryRequest, provision_service_client::ProvisionServiceClient,
};
use crate::console::Console;
use crate::format::time::format_timestamp;

#[derive(Subcommand, Clone)]
pub enum Action {
    History {
        #[arg(long, short, default_value = "10")]
        limit: u32,
    },
}

/// Handles rollback subcommands.
pub async fn handle(channel: Channel, action: Action, console: &Console) -> Result<()> {
    match action {
        Action::History { limit } => history(channel, limit, console).await,
    }
}

async fn history(channel: Channel, limit: u32, console: &Console) -> Result<()> {
    let mut client = ProvisionServiceClient::new(channel);
    let resp = client
        .get_rollback_history(tonic::Request::new(GetRollbackHistoryRequest { limit }))
        .await?
        .into_inner();

    if !resp.error.is_empty() {
        return Err(anyhow::anyhow!("{}", resp.error));
    }

    if resp.entries.is_empty() {
        console.line(Role::Muted, "No rollback history found.");
        return Ok(());
    }

    let table = resp.entries.iter().fold(
        Table::new().header(&["ROLLED BACK AT", "UPDATE ID", "FAILED IMAGE", "REASON"]),
        |table, entry| {
            table.row(&[
                &format_timestamp(entry.rolled_back_at),
                &entry.update_id,
                &entry.failed_version,
                &entry.reason,
            ])
        },
    );

    console.table(&table)?;

    Ok(())
}
