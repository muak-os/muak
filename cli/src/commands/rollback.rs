use anyhow::Result;
use clap::Subcommand;
use tonic::transport::Channel;

use crate::client::provision_service::{
    GetRollbackHistoryRequest, provision_service_client::ProvisionServiceClient,
};
use crate::format::time::{Separator, format_timestamp};
use crate::ui;

#[derive(Subcommand, Clone)]
pub enum Action {
    History {
        #[arg(long, short, default_value = "10")]
        limit: u32,
    },
}

/// Handles rollback subcommands.
pub async fn handle(channel: Channel, action: Action) -> Result<()> {
    match action {
        Action::History { limit } => history(channel, limit).await,
    }
}

async fn history(channel: Channel, limit: u32) -> Result<()> {
    let mut client = ProvisionServiceClient::new(channel);
    let resp = client
        .get_rollback_history(tonic::Request::new(GetRollbackHistoryRequest { limit }))
        .await?
        .into_inner();

    if !resp.error.is_empty() {
        return Err(anyhow::anyhow!("{}", resp.error));
    }

    if resp.entries.is_empty() {
        println!("{}", ui::style::muted("No rollback history found."));
        return Ok(());
    }

    let table = resp.entries.iter().fold(
        ui::table::Table::new().header(&["ROLLED BACK AT", "UPDATE ID", "FAILED IMAGE", "REASON"]),
        |table, entry| {
            table.row(&[
                &format_timestamp(entry.rolled_back_at, Separator::Display),
                &entry.update_id,
                &entry.failed_image,
                &entry.reason,
            ])
        },
    );

    table.print();
    Ok(())
}
