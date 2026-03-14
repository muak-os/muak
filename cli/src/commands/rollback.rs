use anyhow::Result;
use clap::Subcommand;
use tonic::transport::Channel;

use crate::client::{GetRollbackHistoryRequest, ProvisionServiceClient};
use crate::format::{format_timestamp, time::TimeSeparator};
use crate::ui;

#[derive(Subcommand)]
pub enum RollbackAction {
    History {
        #[arg(long, short, default_value = "10")]
        limit: u32,
    },
}

/// Handles rollback subcommands.
pub async fn handle(channel: Channel, action: RollbackAction) -> Result<()> {
    match action {
        RollbackAction::History { limit } => history(channel, limit).await,
    }
}

async fn history(channel: Channel, limit: u32) -> Result<()> {
    let mut client = ProvisionServiceClient::new(channel);
    let resp = client
        .get_rollback_history(tonic::Request::new(GetRollbackHistoryRequest { limit }))
        .await?
        .into_inner();

    if !resp.error.is_empty() {
        eprintln!(
            "{} {}",
            ui::style::error("Error:"),
            ui::style::error_text(&resp.error)
        );
        std::process::exit(1);
    }

    if resp.entries.is_empty() {
        println!("{}", ui::style::muted("No rollback history found."));
        return Ok(());
    }

    let table = resp.entries.iter().fold(
        ui::Table::new().header(&["ROLLED BACK AT", "UPDATE ID", "FAILED IMAGE", "REASON"]),
        |t, e| {
            t.row(&[
                &format_timestamp(e.rolled_back_at, TimeSeparator::Display),
                &e.update_id,
                &e.failed_image,
                &e.reason,
            ])
        },
    );

    table.print();
    Ok(())
}
